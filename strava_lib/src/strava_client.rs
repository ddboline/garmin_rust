use cpython::{
    FromPyObject, ObjectProtocol, PyDict, PyIterator, PyObject, PyResult, PyString, PyTuple,
    Python, PythonObject,
};
use failure::{err_msg, Error};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};
use tempfile::Builder;

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::strava_sync::StravaItem;
use garmin_lib::utils::garmin_util::gzip_file;
use garmin_lib::utils::sport_types::SportTypes;

pub struct LocalStravaItem(pub StravaItem);

impl<'a> FromPyObject<'a> for LocalStravaItem {
    fn extract(py: Python, obj: &'a PyObject) -> PyResult<Self> {
        let start_date = obj.getattr(py, "start_date")?;
        let start_date = start_date.call_method(py, "isoformat", PyTuple::empty(py), None)?;
        let start_date = String::extract(py, &start_date)?;
        let title = obj.getattr(py, "name")?;
        let title = String::extract(py, &title)?;
        let item = StravaItem {
            begin_datetime: start_date.replace("+00:00", "Z"),
            title,
        };
        Ok(LocalStravaItem(item))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum StravaAuthType {
    Read,
    Write,
}

impl Default for StravaAuthType {
    fn default() -> Self {
        StravaAuthType::Read
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
        Default::default()
    }

    pub fn from_file(
        config: &GarminConfig,
        auth_type: Option<StravaAuthType>,
    ) -> Result<Self, Error> {
        let mut client = StravaClient {
            config: config.clone(),
            auth_type,
            ..Default::default()
        };
        let f = File::open(&client.config.strava_tokenfile)?;
        let b = BufReader::new(f);
        for l in b.lines() {
            let line = l?;
            let items: Vec<_> = line.split('=').collect();
            if items.len() >= 2 {
                let key = items[0];
                let val = items[1];
                match key.trim() {
                    "client_id" => client.client_id = val.trim().to_string(),
                    "client_secret" => client.client_secret = val.trim().to_string(),
                    "read_access_token" => client.read_access_token = Some(val.trim().to_string()),
                    "write_access_token" => {
                        client.write_access_token = Some(val.trim().to_string())
                    }
                    _ => {}
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
                Some(ac) => PyTuple::new(py, &[PyString::new(py, ac).into_object()]),
                None => PyTuple::empty(py),
            },
            None,
        )
    }

    fn _get_authorization_url(&self, py: Python) -> PyResult<String> {
        let scope = match self.auth_type {
            Some(StravaAuthType::Read) => "activity:read_all",
            _ => "activity:write",
        };
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
        args.set_item(py, "scope", scope)?;
        args.set_item(py, "state", state)?;
        let result =
            client.call_method(py, "authorization_url", PyTuple::empty(py), Some(&args))?;
        String::extract(py, &result)
    }

    pub fn get_authorization_url(&self) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_authorization_url(py)
            .map_err(|e| err_msg(format!("{:?}", e)))
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
            .map_err(|e| err_msg(format!("{:?}", e)))
            .and_then(|o| o.ok_or_else(|| err_msg("No code received")))?;
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
        start_date: Option<String>,
        end_date: Option<String>,
    ) -> PyResult<HashMap<String, StravaItem>> {
        let client = self.get_strava_client(py)?;
        let args = PyDict::new(py);
        if let Some(start_date) = start_date {
            args.set_item(py, "after", &start_date)?;
        }
        if let Some(end_date) = end_date {
            args.set_item(py, "before", &end_date)?;
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
        start_date: Option<String>,
        end_date: Option<String>,
    ) -> Result<HashMap<String, StravaItem>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_strava_activites(py, start_date, end_date)
            .map_err(|e| err_msg(format!("{:?}", e)))
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
                    PyString::new(py, filepath).into_object(),
                    PyString::new(py, "rb").into_object(),
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
                    PyString::new(py, fext).into_object(),
                    PyString::new(py, title).into_object(),
                    PyString::new(py, description).into_object(),
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
                println!("Error {:?}", e);
            }

            upstat.call_method(py, "poll", PyTuple::empty(py), None)?;
        }

        let activity_id = upstat.getattr(py, "activity_id")?;
        let activity_id = i64::extract(py, &activity_id)?;

        Ok(Some(activity_id.to_string()))
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
            .ok_or_else(|| err_msg("No extension"))?
            .to_str()
            .ok_or_else(|| err_msg("OsStr to Str conversion error"))?
            .to_string();

        let filename = match ext.as_str() {
            "gz" => filepath
                .canonicalize()?
                .to_str()
                .ok_or_else(|| err_msg("OsStr to Str conversion error"))?
                .to_string(),
            _ => {
                let tfile = Builder::new().suffix(&format!("{}.gz", ext)).tempfile()?;
                let infname = filepath
                    .canonicalize()?
                    .to_str()
                    .ok_or_else(|| err_msg("OsStr to Str conversion error"))?
                    .to_string();
                let outfname = tfile
                    .path()
                    .to_str()
                    .ok_or_else(|| err_msg("OsStr to Str conversion error"))?
                    .to_string();
                gzip_file(&infname, &outfname)?;
                _tempfile = Some(tfile);
                outfname
            }
        };

        match self._upload_strava_activity(py, &filename, title, description, is_private, sport) {
            Ok(x) => x.ok_or_else(|| err_msg(format!("Bad extension {}", filename))),
            Err(e) => {
                let err = format!("{:?}", e);
                if err.contains("duplicate of activity") {
                    err.replace("'", " ")
                        .split("duplicate of activity ")
                        .nth(1)
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(0)
                        .map(|x| x.to_string())
                        .ok_or_else(|| err_msg("No id"))
                } else {
                    Err(err_msg(err))
                }
            }
        }
    }
}
