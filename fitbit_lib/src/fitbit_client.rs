use cpython::{
    FromPyObject, ObjectProtocol, PyDict, PyObject, PyResult, PyString, PyTuple, Python,
    PythonObject,
};
use failure::{err_msg, Error};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};

use garmin_lib::common::garmin_config::GarminConfig;

#[derive(Default, Debug)]
pub struct FitbitClient {
    pub config: GarminConfig,
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

macro_rules! set_attr_from_dect {
    ($token:ident, $py:ident, $s:ident, $item:ident) => {
        $token
            .get_item($py, stringify!($item))
            .as_ref()
            .map(|v| String::extract($py, v).map(|x| $s.$item = x))
            .transpose()
    };
}

impl FitbitClient {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn from_file(config: &GarminConfig) -> Result<Self, Error> {
        let mut client = FitbitClient {
            config: config.clone(),
            ..Default::default()
        };
        let f = File::open(&client.config.fitbit_tokenfile)?;
        let b = BufReader::new(f);
        for l in b.lines() {
            let line = l?;
            let items: Vec<_> = line.split('=').collect();
            if items.len() >= 2 {
                let key = items[0];
                let val = items[1];
                match key {
                    "user_id" => client.user_id = val.trim().to_string(),
                    "access_token" => client.access_token = val.trim().to_string(),
                    "refresh_token" => client.refresh_token = val.trim().to_string(),
                    _ => {}
                }
            }
        }
        Ok(client)
    }

    pub fn to_file(&self) -> Result<(), Error> {
        let mut f = File::create(&self.config.fitbit_tokenfile)?;
        writeln!(f, "user_id={}", self.user_id)?;
        writeln!(f, "access_token={}", self.access_token)?;
        writeln!(f, "refresh_token={}", self.refresh_token)?;
        Ok(())
    }

    pub fn get_fitbit_client(&self, py: Python) -> PyResult<PyObject> {
        let redirect_uri = format!("https://{}/garmin/fitbit/callback", self.config.domain);
        let fitbit = py.import("fitbit.api")?;
        let args = PyDict::new(py);
        args.set_item(py, "redirect_uri", redirect_uri)?;
        args.set_item(py, "timeout", 10)?;
        let fitbit_client = fitbit.call(
            py,
            "Fitbit",
            PyTuple::new(
                py,
                &[
                    PyString::new(py, &self.config.fitbit_clientid).into_object(),
                    PyString::new(py, &self.config.fitbit_clientsecret).into_object(),
                ],
            ),
            Some(&args),
        )?;
        fitbit_client.getattr(py, "client")
    }

    fn _get_fitbit_auth_url(&self, py: Python) -> PyResult<String> {
        let client = self.get_fitbit_client(py)?;
        let authorize_token_url = client.getattr(py, "authorize_token_url")?;
        let result = authorize_token_url.call(py, PyTuple::empty(py), None)?;
        let result = PyTuple::extract(py, &result)?;
        let url = result.get_item(py, 0);
        String::extract(py, &url)
    }

    pub fn get_fitbit_auth_url(&self) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_auth_url(py)
            .map_err(|e| err_msg(format!("{:?}", e)))
    }

    fn _get_fitbit_access_token(&mut self, py: Python, code: &str) -> PyResult<String> {
        let client = self.get_fitbit_client(py)?;
        let fetch_access_token = client.getattr(py, "fetch_access_token")?;
        fetch_access_token.call(
            py,
            PyTuple::new(py, &[PyString::new(py, code).into_object()]),
            None,
        )?;
        let session = client.getattr(py, "session")?;
        let token = session.getattr(py, "token")?;
        let token = PyDict::extract(py, &token)?;
        set_attr_from_dect!(token, py, self, user_id)?;
        set_attr_from_dect!(token, py, self, access_token)?;
        set_attr_from_dect!(token, py, self, refresh_token)?;
        let success = r#"
            <h1>You are now authorized to access the Fitbit API!</h1>
            <br/><h3>You can close this window</h3>"#
            .to_string();
        Ok(success)
    }

    pub fn get_fitbit_access_token(&mut self, code: &str) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_access_token(py, code)
            .map_err(|e| err_msg(format!("{:?}", e)))
    }
}

#[cfg(test)]
mod tests {
    use crate::fitbit_client::FitbitClient;
    use garmin_lib::common::garmin_config::GarminConfig;

    #[test]
    fn test_fitbit_client_from_file() {
        let config = GarminConfig::get_config(None).unwrap();
        let client = FitbitClient::from_file(&config).unwrap();
        let url = client.get_fitbit_auth_url().unwrap();
        println!("{:?} {}", client, url);
        assert!(url.len() > 0);
    }
}
