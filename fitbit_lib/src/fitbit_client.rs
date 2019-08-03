use cpython::{
    exc, FromPyObject, ObjectProtocol, PyDict, PyErr, PyList, PyObject, PyResult, PyString,
    PyTuple, Python, PythonObject,
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

fn exception(py: Python, msg: &str) -> PyErr {
    PyErr::new::<exc::Exception, _>(py, msg)
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

macro_rules! get_pydict_item_option {
    ($py:ident, $dict:ident, $id:ident, $T:ty) => {
        $dict
            .get_item($py, &stringify!($id))
            .as_ref()
            .map(|v| <$T>::extract($py, v))
            .transpose()
    };
}

macro_rules! get_pydict_item {
    ($py:ident, $dict:ident, $id:ident, $T:ty) => {
        get_pydict_item_option!($py, $dict, $id, $T)
            .and_then(|x| x.ok_or_else(|| exception($py, &format!("No {}", stringify!($id)))))
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
        let result = client.call_method(py, "authorize_token_url", PyTuple::empty(py), None)?;
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
        client.call_method(
            py,
            "fetch_access_token",
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

    fn _get_fitbit_intraday_time_series_heartrate(
        &self,
        py: Python,
        date: &str,
    ) -> PyResult<Vec<HeartRateEntry>> {
        let client = self.get_fitbit_client(py)?;
        client.call_method(py, "user_profile_get", PyTuple::empty(py), None)?;
        let args = PyDict::new(py);
        args.set_item(py, "base_date", date)?;
        let result = client.call_method(
            py,
            "intraday_time_series",
            PyTuple::new(py, &[PyString::new(py, "activities/heart").into_object()]),
            Some(&args),
        )?;
        let activities_heart_intraday = result.get_item(
            py,
            PyString::new(py, "activities-heart-intraday").into_object(),
        )?;
        let dataset = activities_heart_intraday.get_item(py, "dataset")?;
        let dataset = PyList::extract(py, &dataset)?;
        let mut results = Vec::new();
        for item in dataset.iter(py) {
            let dict = PyDict::extract(py, &item)?;
            results.push(HeartRateEntry::from_pydict(py, dict)?);
        }
        Ok(results)
    }

    pub fn get_fitbit_intraday_time_series_heartrate(
        &self,
        date: &str,
    ) -> Result<Vec<HeartRateEntry>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_intraday_time_series_heartrate(py, date)
            .map_err(|e| err_msg(format!("{:?}", e)))
    }
}

#[derive(Serialize, Deserialize)]
pub struct HeartRateEntry {
    pub time: String,
    pub value: String,
}

impl HeartRateEntry {
    pub fn from_pydict(py: Python, dict: PyDict) -> PyResult<Self> {
        let time = get_pydict_item!(py, dict, time, String)?;
        let value = get_pydict_item!(py, dict, value, String)?;
        let hre = Self { time, value };
        Ok(hre)
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
