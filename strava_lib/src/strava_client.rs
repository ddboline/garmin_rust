use anyhow::{format_err, Error};
use base64::{encode, encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, SecondsFormat, Utc};
use cpython::{
    exc, FromPyObject, ObjectProtocol, PyDict, PyErr, PyIterator, PyObject, PyResult, PyTuple,
    Python, PythonObject, ToPyObject,
};
use lazy_static::lazy_static;
use log::debug;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Url};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    time::{Duration, SystemTime},
};
use tempfile::Builder;
use tokio::sync::Mutex;

use garmin_lib::{
    common::{garmin_config::GarminConfig, strava_sync::StravaItem},
    utils::{
        garmin_util::gzip_file, iso_8601_datetime::{self, convert_str_to_datetime},
        sport_types::SportTypes, stack_string::StackString,
    },
};

lazy_static! {
    static ref CSRF_TOKEN: Mutex<Option<StackString>> = Mutex::new(None);
}

fn exception(py: Python, msg: &str) -> PyErr {
    PyErr::new::<exc::Exception, _>(py, msg)
}

pub struct LocalStravaItem(pub StravaItem);

impl<'a> FromPyObject<'a> for LocalStravaItem {
    fn extract(py: Python, obj: &'a PyObject) -> PyResult<Self> {
        let start_date = obj.getattr(py, "start_date")?;
        let start_date = start_date.call_method(py, "isoformat", PyTuple::empty(py), None)?;
        let start_date = String::extract(py, &start_date)?;
        let title = obj.getattr(py, "name")?;
        let title = String::extract(py, &title)?;
        let item = StravaItem {
            begin_datetime: convert_str_to_datetime(&start_date.replace("+00:00", "Z"))
                .map_err(|e| exception(py, &e.to_string()))?,
            title: title.into(),
        };
        Ok(Self(item))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum StravaAuthType {
    Read,
    Write,
}

impl Default for StravaAuthType {
    fn default() -> Self {
        Self::Read
    }
}

#[derive(Default, Debug)]
pub struct StravaClient {
    pub config: GarminConfig,
    pub auth_type: Option<StravaAuthType>,
    pub client_id: StackString,
    pub client_secret: StackString,
    pub read_access_token: Option<StackString>,
    pub write_access_token: Option<StackString>,
    pub read_refresh_token: Option<StackString>,
    pub client: Client,
}

impl StravaClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_file(
        config: GarminConfig,
        auth_type: Option<StravaAuthType>,
    ) -> Result<Self, Error> {
        let mut client = Self {
            config,
            auth_type,
            ..Self::default()
        };
        let f = File::open(client.config.strava_tokenfile.as_str())?;
        let mut b = BufReader::new(f);
        let mut line = String::new();
        loop {
            line.clear();
            if b.read_line(&mut line)? == 0 {
                break;
            }
            let mut items = line.split('=');
            if let Some(key) = items.next() {
                if let Some(val) = items.next() {
                    match key.trim() {
                        "client_id" => client.client_id = val.trim().into(),
                        "client_secret" => client.client_secret = val.trim().into(),
                        "read_access_token" => client.read_access_token = Some(val.trim().into()),
                        "write_access_token" => client.write_access_token = Some(val.trim().into()),
                        _ => {}
                    }
                }
            }
        }
        Ok(client)
    }

    pub fn to_file(&self) -> Result<(), Error> {
        let mut f = File::create(self.config.strava_tokenfile.as_str())?;
        writeln!(f, "[API]")?;
        writeln!(f, "client_id = {}", self.client_id)?;
        writeln!(f, "client_secret = {}", self.client_secret)?;
        if let Some(token) = self.read_access_token.as_ref() {
            writeln!(f, "read_access_token = {}", token)?;
        }
        if let Some(token) = self.read_refresh_token.as_ref() {
            writeln!(f, "read_refresh_token = {}", token)?;
        }
        if let Some(token) = self.write_access_token.as_ref() {
            writeln!(f, "write_access_token = {}", token)?;
        }
        Ok(())
    }

    pub fn get_strava_client(&self, py: Python) -> PyResult<PyObject> {
        let stravalib = py.import("stravalib")?;
        let access_token = self.auth_type.and_then(|t| match t {
            StravaAuthType::Read => self.read_access_token.as_ref(),
            StravaAuthType::Write => self.write_access_token.as_ref(),
        });
        stravalib.call(
            py,
            "Client",
            match access_token {
                Some(ac) => PyTuple::new(py, &[ac.as_str().to_py_object(py).into_object()]),
                None => PyTuple::empty(py),
            },
            None,
        )
    }

    fn _get_authorization_url(&self, py: Python) -> PyResult<String> {
        let state = match self.auth_type {
            Some(StravaAuthType::Read) => "YWN0aXZpdHk6cmVhZF9hbGw=",
            _ => "YWN0aXZpdHk6d3JpdGU=",
        };
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        args.set_item(py, "client_id", self.client_id.as_str())?;
        args.set_item(
            py,
            "redirect_uri",
            &format!("https://{}/garmin/strava/callback", &self.config.domain),
        )?;
        match self.auth_type {
            Some(StravaAuthType::Read) => args.set_item(py, "scope", "activity:read_all")?,
            _ => args.set_item(
                py,
                "scope",
                PyTuple::new(
                    py,
                    &[
                        "activity:write".to_py_object(py).into_object(),
                        "activity:read_all".to_py_object(py).into_object(),
                    ],
                ),
            )?,
        };

        args.set_item(py, "state", state)?;
        let result =
            client.call_method(py, "authorization_url", PyTuple::empty(py), Some(&args))?;
        String::extract(py, &result)
    }

    pub fn get_authorization_url(&self) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_authorization_url(py)
            .map_err(|e| format_err!("{:?}", e))
    }

    fn get_random_string() -> String {
        let random_bytes: Vec<u8> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD)
    }

    pub async fn get_authorization_url_api(&self) -> Result<Url, Error> {
        let redirect_uri = format!("https://{}/garmin/strava/callback", &self.config.domain);
        let state = Self::get_random_string();
        let url = Url::parse_with_params(
            "https://www.strava.com/oauth/authorize",
            &[
                ("client_id", self.client_id.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("response_type", "code"),
                ("approval_prompt", "auto"),
                ("scope", "activity:read_all,activity:write"),
                ("state", state.as_str()),
            ],
        )?;
        CSRF_TOKEN.lock().await.replace(state.into());
        Ok(url)
    }

    fn _exchange_code_for_token(&self, py: Python, code: &str) -> PyResult<Option<String>> {
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        args.set_item(py, "client_id", self.client_id.as_str())?;
        args.set_item(py, "client_secret", self.client_secret.as_str())?;
        args.set_item(py, "code", code)?;
        let result = client.call_method(
            py,
            "exchange_code_for_token",
            PyTuple::empty(py),
            Some(&args),
        )?;
        let result = PyDict::extract(py, &result)?;
        result
            .get_item(py, "access_token")
            .as_ref()
            .map(|v| String::extract(py, v))
            .transpose()
    }

    pub fn process_callback(&mut self, code: &str, state: &str) -> Result<(), Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        let code = self
            ._exchange_code_for_token(py, code)
            .map_err(|e| format_err!("{:?}", e))
            .and_then(|o| o.ok_or_else(|| format_err!("No code received")))?
            .into();
        self.auth_type = match state {
            "YWN0aXZpdHk6cmVhZF9hbGw=" => {
                self.read_access_token = Some(code);
                Some(StravaAuthType::Read)
            }
            "YWN0aXZpdHk6d3JpdGU=" => {
                self.write_access_token = Some(code);
                Some(StravaAuthType::Write)
            }
            _ => None,
        };
        Ok(())
    }

    pub async fn process_callback_api(&mut self, code: &str, state: &str) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: StackString,
            refresh_token: StackString,
        }

        let current_state = CSRF_TOKEN.lock().await.take();
        if let Some(current_state) = current_state {
            if state != current_state.as_str() {
                return Err(format_err!("Incorrect state"));
            }
            let url = "https://www.strava.com/oauth/token";
            let data = hashmap! {
                "client_id" => self.client_id.as_str(),
                "client_secret" => self.client_secret.as_str(),
                "code" => code,
                "grant_type" => "authorization_code",
            };
            let resp: TokenResponse = self
                .client
                .post(url)
                .form(&data)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            self.read_access_token.replace(resp.access_token);
            self.read_refresh_token.replace(resp.refresh_token);
            Ok(())
        } else {
            Err(format_err!("No state"))
        }
    }

    pub async fn refresh_access_token(&mut self) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: StackString,
            refresh_token: StackString,
        }

        if let Some(refresh_token) = self.read_refresh_token.as_ref() {
            let url = "https://www.strava.com/oauth/token";
            let data = hashmap! {
                "client_id" => self.client_id.as_str(),
                "client_secret" => self.client_secret.as_str(),
                "refresh_token" => refresh_token.as_str(),
                "grant_type" => "refresh_token",
            };
            let resp: TokenResponse = self
                .client
                .post(url)
                .form(&data)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            self.read_access_token.replace(resp.access_token);
            self.read_refresh_token.replace(resp.refresh_token);
            Ok(())
        } else {
            Err(format_err!("No refresh token"))
        }
    }

    fn get_auth_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        let access_token = self
            .read_access_token
            .as_ref()
            .or_else(|| self.write_access_token.as_ref())
            .ok_or_else(|| format_err!("no access token"))?;
        headers.insert("Authorization", format!("Bearer {}", access_token).parse()?);
        Ok(headers)
    }

    fn _get_strava_activites(
        &self,
        py: Python,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> PyResult<HashMap<StackString, StravaItem>> {
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        if let Some(start_date) = start_date {
            args.set_item(
                py,
                "after",
                start_date.to_rfc3339_opts(SecondsFormat::Secs, true),
            )?;
        }
        if let Some(end_date) = end_date {
            args.set_item(
                py,
                "before",
                end_date.to_rfc3339_opts(SecondsFormat::Secs, true),
            )?;
        }
        let activities =
            client.call_method(py, "get_activities", PyTuple::empty(py), Some(&args))?;
        let activities = PyIterator::from_object(py, activities)?;

        let mut results = HashMap::new();

        for activity in activities {
            let activity = activity?;
            let id = activity.getattr(py, "id")?;
            let id = i64::extract(py, &id)?.to_string().into();
            let item = LocalStravaItem::extract(py, &activity)?;
            results.insert(id, item.0);
        }
        Ok(results)
    }

    pub fn get_strava_activites(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> Result<HashMap<StackString, StravaItem>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_strava_activites(py, start_date, end_date)
            .map_err(|e| format_err!("{:?}", e))
    }

    pub async fn get_strava_activites_api(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> Result<HashMap<StackString, StravaItem>, Error> {
        #[derive(Deserialize)]
        struct StravaActivity {
            name: String,
            #[serde(with = "iso_8601_datetime")]
            start_date: DateTime<Utc>,
            id: i64,
        }

        let mut params = Vec::new();
        if let Some(start_date) = start_date {
            params.push(("after", start_date.to_rfc3339_opts(SecondsFormat::Secs, true)));
        }
        if let Some(end_date) = end_date {
            params.push(("before", end_date.to_rfc3339_opts(SecondsFormat::Secs, true)));
        }

        let headers = self.get_auth_headers()?;
        let url = Url::parse_with_params(
            "https://www.strava.com/api/v3/athlete/activities",
            &params,
        )?;
        let activities: Vec<StravaActivity> = self.client.get(url).headers(headers).send().await?.error_for_status()?.json().await?;
        let activity_map: HashMap<_, _> = activities.into_iter().map(|act| {
            (act.id.to_string().into(), StravaItem {
                begin_datetime: act.start_date,
                title: act.name.into(),
            })
        }).collect();
        Ok(activity_map)
    }

    fn _upload_strava_activity(
        &self,
        py: Python,
        filepath: &str,
        title: &str,
        description: &str,
        is_private: bool,
        sport: SportTypes,
    ) -> PyResult<Option<String>> {
        let fext = if filepath.ends_with("fit.gz") {
            "fit.gz"
        } else if filepath.ends_with("tcx.gz") {
            "tcx.gz"
        } else {
            return Ok(None);
        };
        let client = self.get_strava_client(py)?;
        let builtins = py.import("builtins")?;
        let file_obj = builtins.call(
            py,
            "open",
            PyTuple::new(
                py,
                &[
                    filepath.to_py_object(py).into_object(),
                    "rb".to_py_object(py).into_object(),
                ],
            ),
            None,
        )?;
        let args = PyDict::new(py);
        args.set_item(py, "private", is_private)?;
        args.set_item(py, "activity_type", sport.to_strava_activity())?;
        let upstat = client.call_method(
            py,
            "upload_activity",
            PyTuple::new(
                py,
                &[
                    file_obj,
                    fext.to_py_object(py).into_object(),
                    title.to_py_object(py).into_object(),
                    description.to_py_object(py).into_object(),
                ],
            ),
            Some(&args),
        )?;

        let start_time = SystemTime::now();
        let timeout = Duration::from_secs(10);

        loop {
            let result = String::extract(py, &upstat.getattr(py, "activity_id")?);

            if result.is_ok()
                || SystemTime::now()
                    .duration_since(start_time)
                    .unwrap_or_else(|_| Duration::from_secs(20))
                    > timeout
            {
                break;
            }

            if let Err(e) = result {
                debug!("Error {:?}", e);
            }

            upstat.call_method(py, "poll", PyTuple::empty(py), None)?;
        }

        let activity_id = upstat.getattr(py, "activity_id")?;
        let activity_id = i64::extract(py, &activity_id)?;

        let url = format!(
            "https://www.strava.com/activities/{}",
            activity_id.to_string()
        );
        Ok(Some(url))
    }

    pub fn upload_strava_activity(
        &self,
        filepath: &Path,
        title: &str,
        description: &str,
        is_private: bool,
        sport: SportTypes,
    ) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        let mut _tempfile: Option<_> = None;

        let ext = filepath
            .extension()
            .ok_or_else(|| format_err!("No extension"))?
            .to_string_lossy()
            .to_string();

        let filename = if &ext == "gz" {
            filepath.canonicalize()?.to_string_lossy().to_string()
        } else {
            let tfile = Builder::new().suffix(&format!("{}.gz", ext)).tempfile()?;
            let infname = filepath.canonicalize()?.to_string_lossy().to_string();
            let outfname = tfile.path().to_string_lossy().to_string();
            gzip_file(&infname, &outfname)?;
            _tempfile = Some(tfile);
            outfname
        };

        match self._upload_strava_activity(py, &filename, title, description, is_private, sport) {
            Ok(x) => x.ok_or_else(|| format_err!("Bad extension {}", filename)),
            Err(e) => {
                let err = format!("{:?}", e);
                if err.contains("duplicate of activity") {
                    err.replace("'", " ")
                        .split("duplicate of activity ")
                        .nth(1)
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .map(ToString::to_string)
                        .ok_or_else(|| format_err!("No id"))
                } else {
                    Err(format_err!(err))
                }
            }
        }
    }

    pub async fn upload_strava_activity_api(&self,
        filepath: &Path,
        title: &str,
        description: &str,
        is_private: bool,
        sport: SportTypes) -> Result<String, Error> {
            Ok("".to_string())
        }

    fn _update_strava_activity(
        &self,
        py: Python,
        activity_id: i64,
        title: &str,
        description: Option<&str>,
        is_private: Option<bool>,
        sport: SportTypes,
    ) -> PyResult<String> {
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        args.set_item(py, "name", title)?;
        args.set_item(py, "activity_type", sport.to_strava_activity())?;
        if let Some(is_private) = is_private {
            args.set_item(py, "private", is_private)?;
        }
        if let Some(description) = description {
            args.set_item(py, "description", description)?;
        }
        client.call_method(
            py,
            "update_activity",
            PyTuple::new(py, &[activity_id.to_py_object(py).into_object()]),
            Some(&args),
        )?;
        let url = format!("https://{}/garmin/strava_sync", self.config.domain);
        Ok(url)
    }

    pub fn update_strava_activity(
        &self,
        activity_id: &str,
        title: &str,
        description: Option<&str>,
        is_private: Option<bool>,
        sport: SportTypes,
    ) -> Result<String, Error> {
        let activity_id: i64 = activity_id.parse()?;

        let gil = Python::acquire_gil();
        let py = gil.python();

        self._update_strava_activity(py, activity_id, title, description, is_private, sport)
            .map_err(|e| format_err!("{:?}", e))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use garmin_lib::common::garmin_config::GarminConfig;

    use crate::strava_client::{StravaAuthType, StravaClient};

    #[tokio::test]
    #[ignore]
    async fn test_get_strava_activites_api() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::from_file(config, Some(StravaAuthType::Read))?;
        let activities = client.get_strava_activites_api(None, None).await?;
        println!("{:#?}", activities);
        assert!(false);
        Ok(())
    }
}