use anyhow::{format_err, Error};
use chrono::{DateTime, SecondsFormat, Utc};
use cpython::{
    exc, FromPyObject, ObjectProtocol, PyDict, PyErr, PyIterator, PyObject, PyResult, PyTuple,
    Python, PythonObject, ToPyObject,
};
use log::debug;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    time::{Duration, SystemTime},
};
use tempfile::Builder;

use garmin_lib::{
    common::{garmin_config::GarminConfig, strava_sync::StravaItem},
    utils::{
        garmin_util::gzip_file, iso_8601_datetime::convert_str_to_datetime, sport_types::SportTypes,
    },
};

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
            title,
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
    pub client_id: String,
    pub client_secret: String,
    pub read_access_token: Option<String>,
    pub write_access_token: Option<String>,
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
        let f = File::open(&client.config.strava_tokenfile)?;
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
                        "client_id" => client.client_id = val.trim().to_string(),
                        "client_secret" => client.client_secret = val.trim().to_string(),
                        "read_access_token" => {
                            client.read_access_token = Some(val.trim().to_string())
                        }
                        "write_access_token" => {
                            client.write_access_token = Some(val.trim().to_string())
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(client)
    }

    pub fn to_file(&self) -> Result<(), Error> {
        let mut f = File::create(&self.config.strava_tokenfile)?;
        writeln!(f, "[API]")?;
        writeln!(f, "client_id = {}", self.client_id)?;
        writeln!(f, "client_secret = {}", self.client_secret)?;
        if let Some(token) = self.read_access_token.as_ref() {
            writeln!(f, "read_access_token = {}", token)?;
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
                Some(ac) => PyTuple::new(py, &[ac.to_py_object(py).into_object()]),
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
        args.set_item(py, "client_id", &self.client_id)?;
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

    fn _exchange_code_for_token(&self, py: Python, code: &str) -> PyResult<Option<String>> {
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        args.set_item(py, "client_id", &self.client_id)?;
        args.set_item(py, "client_secret", &self.client_secret)?;
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
            .and_then(|o| o.ok_or_else(|| format_err!("No code received")))?;
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

    fn _get_strava_activites(
        &self,
        py: Python,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> PyResult<HashMap<String, StravaItem>> {
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
            let id = i64::extract(py, &id)?.to_string();
            let item = LocalStravaItem::extract(py, &activity)?;
            results.insert(id, item.0);
        }
        Ok(results)
    }

    pub fn get_strava_activites(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> Result<HashMap<String, StravaItem>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_strava_activites(py, start_date, end_date)
            .map_err(|e| format_err!("{:?}", e))
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

        let filename = if ext.as_str() == "gz" {
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
