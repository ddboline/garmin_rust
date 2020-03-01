use anyhow::{format_err, Error};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use cpython::{
    FromPyObject, ObjectProtocol, PyDict, PyList, PyObject, PyResult, PyTuple, Python,
    PythonObject, ToPyObject,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
};

use garmin_lib::common::garmin_config::GarminConfig;

use crate::{
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    scale_measurement::ScaleMeasurement,
};

#[derive(Default, Debug, Clone)]
pub struct FitbitClient {
    pub config: GarminConfig,
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

macro_rules! set_attr_from_dict {
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
        Self::default()
    }

    pub fn from_file(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self {
            config,
            ..Self::default()
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
                    "user_id" => client.user_id = val.trim().into(),
                    "access_token" => client.access_token = val.trim().into(),
                    "refresh_token" => client.refresh_token = val.trim().into(),
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

    fn get_fitbit_client(&self, py: Python, do_auth: bool) -> PyResult<PyObject> {
        let redirect_uri = format!("https://{}/garmin/fitbit/callback", self.config.domain);
        let fitbit = py.import("fitbit.api")?;
        let args = PyDict::new(py);
        if do_auth {
            args.set_item(py, "redirect_uri", redirect_uri)?;
            args.set_item(py, "timeout", 10)?;
        } else {
            args.set_item(py, "access_token", &self.access_token)?;
            args.set_item(py, "refresh_token", &self.refresh_token)?;
        }
        fitbit.call(
            py,
            "Fitbit",
            PyTuple::new(
                py,
                &[
                    self.config.fitbit_clientid.to_py_object(py).into_object(),
                    self.config
                        .fitbit_clientsecret
                        .to_py_object(py)
                        .into_object(),
                ],
            ),
            Some(&args),
        )
    }

    fn get_client_offset(py: Python, client: &PyObject) -> PyResult<FixedOffset> {
        let result = client
            .call_method(py, "user_profile_get", PyTuple::empty(py), None)?
            .get_item(py, "user")?;
        let result = PyDict::extract(py, &result)?;
        let offset = match result.get_item(py, "offsetFromUTCMillis") {
            Some(r) => i64::extract(py, &r)?,
            None => 0,
        };
        let offset = (offset / 1000) as i32;
        let offset = FixedOffset::east(offset);
        Ok(offset)
    }

    fn _get_fitbit_auth_url(&self, py: Python) -> PyResult<String> {
        let client = self.get_fitbit_client(py, true)?;
        let client = client.getattr(py, "client")?;
        let result = client.call_method(py, "authorize_token_url", PyTuple::empty(py), None)?;
        let result = PyTuple::extract(py, &result)?;
        let url = result.get_item(py, 0);
        String::extract(py, &url)
    }

    pub fn get_fitbit_auth_url(&self) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_auth_url(py)
            .map_err(|e| format_err!("{:?}", e))
    }

    fn _get_fitbit_access_token(&mut self, py: Python, code: &str) -> PyResult<String> {
        let client = self.get_fitbit_client(py, true)?;
        let client = client.getattr(py, "client")?;
        client.call_method(
            py,
            "fetch_access_token",
            PyTuple::new(py, &[code.to_py_object(py).into_object()]),
            None,
        )?;
        let session = client.getattr(py, "session")?;
        let token = session.getattr(py, "token")?;
        let token = PyDict::extract(py, &token)?;
        set_attr_from_dict!(token, py, self, user_id)?;
        set_attr_from_dict!(token, py, self, access_token)?;
        set_attr_from_dict!(token, py, self, refresh_token)?;
        let success = r#"
            <h1>You are now authorized to access the Fitbit API!</h1>
            <br/><h3>You can close this window</h3>
            <script language="JavaScript" type="text/javascript">window.close()</script>
            "#
        .into();
        Ok(success)
    }

    pub fn get_fitbit_access_token(&mut self, code: &str) -> Result<String, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_access_token(py, code)
            .map_err(|e| format_err!("{:?}", e))
    }

    fn _get_fitbit_intraday_time_series_heartrate(
        &self,
        py: Python,
        date: NaiveDate,
    ) -> PyResult<Vec<FitbitHeartRate>> {
        let client = self.get_fitbit_client(py, false)?;
        let offset = Self::get_client_offset(py, &client)?;
        let args = PyDict::new(py);
        let date = date.to_string();
        args.set_item(py, "base_date", &date)?;
        let result = client.call_method(
            py,
            "intraday_time_series",
            ("activities/heart",),
            Some(&args),
        )?;
        let activities_heart_intraday = result.get_item(
            py,
            "activities-heart-intraday".to_py_object(py).into_object(),
        )?;
        let dataset = activities_heart_intraday.get_item(py, "dataset")?;
        let dataset = PyList::extract(py, &dataset)?;

        dataset
            .iter(py)
            .map(|item| {
                let dict = PyDict::extract(py, &item)?;
                FitbitHeartRate::from_pydict(py, &dict, &date, offset)
            })
            .collect()
    }

    pub fn get_fitbit_intraday_time_series_heartrate(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<FitbitHeartRate>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_intraday_time_series_heartrate(py, date)
            .map_err(|e| format_err!("{:?}", e))
    }

    pub fn import_fitbit_heartrate(
        &self,
        date: NaiveDate,
        config: &GarminConfig,
    ) -> Result<(), Error> {
        let heartrates = self.get_fitbit_intraday_time_series_heartrate(date)?;
        FitbitHeartRate::merge_slice_to_avro(config, &heartrates)
    }

    fn _get_fitbit_bodyweightfat(&self, py: Python) -> PyResult<Vec<FitbitBodyWeightFat>> {
        let client = self.get_fitbit_client(py, false)?;
        let offset = Self::get_client_offset(py, &client)?;
        let args = PyDict::new(py);
        args.set_item(py, "period", "30d")?;
        let result = client.call_method(py, "get_bodyweight", PyTuple::empty(py), Some(&args))?;
        let dataset = result.get_item(py, "weight".to_py_object(py).into_object())?;
        let dataset = PyList::extract(py, &dataset)?;

        dataset
            .iter(py)
            .map(|item| {
                let dict = PyDict::extract(py, &item)?;
                FitbitBodyWeightFat::from_pydict(py, &dict, offset)
            })
            .collect()
    }

    pub fn get_fitbit_bodyweightfat(&self) -> Result<Vec<FitbitBodyWeightFat>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._get_fitbit_bodyweightfat(py)
            .map_err(|e| format_err!("{:?}", e))
    }

    fn _update_fitbit_bodyweightfat(
        &self,
        py: Python,
        updates: &[ScaleMeasurement],
    ) -> PyResult<()> {
        let client = self.get_fitbit_client(py, false)?;
        let offset = Self::get_client_offset(py, &client)?;
        updates
            .iter()
            .map(|update| {
                let datetime = update.datetime.with_timezone(&offset);
                let date = datetime.date().naive_local();
                let time = datetime.naive_local().format("%H:%M:%S").to_string();
                let url = "https://api.fitbit.com/1/user/-/body/log/weight.json";
                let data = PyDict::new(py);
                data.set_item(py, "date", &date.to_string())?;
                data.set_item(py, "time", &time)?;
                data.set_item(py, "weight", &update.mass.to_string())?;
                let args = PyDict::new(py);
                args.set_item(py, "data", data)?;
                args.set_item(py, "method", "POST")?;
                client.call_method(py, "make_request", (url,), Some(&args))?;
                let url = "https://api.fitbit.com/1/user/-/body/log/fat.json";
                let data = PyDict::new(py);
                data.set_item(py, "date", &date.to_string())?;
                data.set_item(py, "time", &time)?;
                data.set_item(py, "fat", &update.fat_pct.to_string())?;
                let args = PyDict::new(py);
                args.set_item(py, "data", data)?;
                args.set_item(py, "method", "POST")?;
                client.call_method(py, "make_request", (url,), Some(&args))?;
                Ok(())
            })
            .collect()
    }

    pub fn update_fitbit_bodyweightfat(
        &self,
        updates: Vec<ScaleMeasurement>,
    ) -> Result<Vec<ScaleMeasurement>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        self._update_fitbit_bodyweightfat(py, &updates)
            .map_err(|e| format_err!("{:?}", e))?;
        Ok(updates)
    }

    pub fn _get_tcx_urls(
        &self,
        py: Python,
        start_date: NaiveDate,
    ) -> PyResult<Vec<(DateTime<Utc>, String)>> {
        let client = self.get_fitbit_client(py, false)?;
        let url = format!(
            "https://api.fitbit.com/1/user/-/activities/list.json?afterDate={}&offset=0&limit=20&sort=asc",
            start_date,
        );
        let args = PyDict::new(py);
        args.set_item(py, "method", "GET")?;
        let result = client.call_method(py, "make_request", (&url,), Some(&args))?;
        let result = PyDict::extract(py, &result)?;
        let activities = result.get_item(py, "activities").unwrap();
        let activities = PyList::extract(py, &activities)?;

        activities
            .iter(py)
            .filter_map(|item| {
                let res = || {
                    let dict = PyDict::extract(py, &item)?;
                    let log_type = match dict.get_item(py, "logType").as_ref() {
                        Some(l) => String::extract(py, l)?,
                        None => return Ok(None),
                    };
                    if log_type != "tracker" {
                        return Ok(None);
                    }
                    let start_time = match dict.get_item(py, "startTime").as_ref() {
                        Some(t) => {
                            let start_time = String::extract(py, t)?;
                            DateTime::parse_from_rfc3339(&start_time)
                                .unwrap()
                                .with_timezone(&Utc)
                        }
                        None => return Ok(None),
                    };
                    match dict.get_item(py, "tcxLink").as_ref() {
                        Some(l) => Ok(Some((start_time, String::extract(py, l)?))),
                        None => Ok(None),
                    }
                };
                res().transpose()
            })
            .collect()
    }

    pub fn get_tcx_urls(
        &self,
        start_date: NaiveDate,
    ) -> Result<Vec<(DateTime<Utc>, String)>, Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        self._get_tcx_urls(py, start_date)
            .map_err(|e| format_err!("{:?}", e))
    }

    pub fn _download_tcx(&self, py: Python, tcx_url: &str) -> PyResult<Vec<u8>> {
        let client = self.get_fitbit_client(py, false)?;
        let system = String::extract(py, &client.getattr(py, "system")?)?;
        let client = client.getattr(py, "client")?;
        let headers = PyDict::new(py);
        headers.set_item(py, "Accept-Language", &system)?;
        let args = PyDict::new(py);
        args.set_item(py, "headers", &headers)?;
        args.set_item(py, "method", "GET")?;
        let resp = client.call_method(py, "make_request", (tcx_url,), Some(&args))?;
        resp.call_method(py, "raise_for_status", PyTuple::new(py, &[]), None)?;
        let data = resp.getattr(py, "content")?;
        <Vec<u8>>::extract(py, &data)
    }

    pub fn download_tcx<T: Write>(&self, tcx_url: &str, outfile: &mut T) -> Result<(), Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let data = self
            ._download_tcx(py, tcx_url)
            .map_err(|e| format_err!("{:?}", e))?;
        outfile.write_all(&data).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate};
    use std::{
        io::{stdout, Write},
        path::Path,
    };
    use tempfile::NamedTempFile;

    use crate::fitbit_client::FitbitClient;
    use garmin_lib::common::garmin_config::GarminConfig;

    #[test]
    #[ignore]
    fn test_fitbit_client_from_file() {
        let config = GarminConfig::get_config(None).unwrap();
        let client = FitbitClient::from_file(config).unwrap();
        let url = client.get_fitbit_auth_url().unwrap();
        writeln!(stdout(), "{:?} {}", client, url).unwrap();
        assert!(url.len() > 0);
    }

    #[test]
    #[ignore]
    fn test_get_tcx_urls() {
        let config = GarminConfig::get_config(None).unwrap();
        let client = FitbitClient::from_file(config.clone()).unwrap();
        let start_date = NaiveDate::from_ymd(2019, 12, 1);
        let results = client.get_tcx_urls(start_date).unwrap();
        writeln!(stdout(), "{:?}", results).unwrap();
        for (start_time, tcx_url) in results {
            let fname = format!(
                "{}/{}.tcx",
                config.gps_dir,
                start_time
                    .with_timezone(&Local)
                    .format("%Y-%m-%d_%H-%M-%S_1_1")
                    .to_string(),
            );
            if Path::new(&fname).exists() {
                writeln!(stdout(), "{} exists", fname).unwrap();
            } else {
                writeln!(stdout(), "{} does not exist", fname).unwrap();
            }

            let mut f = NamedTempFile::new().unwrap();
            client.download_tcx(&tcx_url, &mut f).unwrap();
            let metadata = f.as_file().metadata().unwrap();
            writeln!(stdout(), "{} {:?} {}", start_time, metadata, metadata.len()).unwrap();
            assert!(metadata.len() > 0);
            break;
        }
    }
}
