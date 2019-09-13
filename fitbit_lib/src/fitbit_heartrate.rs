use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDateTime, TimeZone, Utc};
use cpython::{exc, FromPyObject, PyDict, PyErr, PyResult, Python};
use failure::Error;
use glob::glob;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{self, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::row_index_trait::RowIndexTrait;

fn exception(py: Python, msg: &str) -> PyErr {
    PyErr::new::<exc::Exception, _>(py, msg)
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

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartRate {
    pub datetime: DateTime<Utc>,
    pub value: i32,
}

#[derive(Deserialize)]
pub struct JsonHeartRateValue {
    pub bpm: i32,
    pub confidence: i32,
}

#[derive(Deserialize)]
pub struct JsonHeartRateEntry {
    #[serde(alias = "dateTime", deserialize_with = "deserialize_json_mdyhms")]
    pub datetime: DateTime<Utc>,
    pub value: JsonHeartRateValue,
}

pub fn deserialize_json_mdyhms<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).and_then(|s| {
        NaiveDateTime::parse_from_str(&s, "%m/%d/%y %H:%M:%S")
            .map(|datetime| {
                let offset = Local.offset_from_utc_datetime(&datetime);
                DateTime::<FixedOffset>::from_utc(datetime, offset).with_timezone(&Utc)
            })
            .map_err(serde::de::Error::custom)
    })
}

impl FitbitHeartRate {
    pub fn from_pydict(
        py: Python,
        dict: PyDict,
        date: &str,
        offset: FixedOffset,
    ) -> PyResult<Self> {
        let time = get_pydict_item!(py, dict, time, String)?;
        let datetime = format!("{}T{}{}", date, time, offset);
        let datetime = DateTime::parse_from_rfc3339(&datetime)
            .unwrap()
            .with_timezone(&Utc);
        let value = get_pydict_item!(py, dict, value, i32)?;
        let hre = Self { datetime, value };
        Ok(hre)
    }

    pub fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let conn = pool.get()?;
        let query = "INSERT INTO fitbit_heartrate (datetime, bpm) VALUES ($1, $2)";
        conn.execute(query, &[&self.datetime, &self.value])?;
        Ok(())
    }

    pub fn from_json_heartrate_entry(entry: JsonHeartRateEntry) -> Self {
        Self {
            datetime: entry.datetime,
            value: entry.value.bpm,
        }
    }

    pub fn read_from_db(pool: &PgPool, date: &str) -> Result<Vec<Self>, Error> {
        let query = format!(
            "
            SELECT datetime, bpm
            FROM fitbit_heartrate
            WHERE date(datetime) = '{}'",
            date
        );
        let conn = pool.get()?;
        conn.query(&query, &[])?
            .iter()
            .map(|row| {
                let datetime = row.get_idx(0)?;
                let value = row.get_idx(1)?;
                Ok(Self { datetime, value })
            })
            .collect()
    }
}

pub fn process_fitbit_json_file(fname: &Path) -> Result<Vec<FitbitHeartRate>, Error> {
    let f = File::open(fname)?;
    let result: Vec<JsonHeartRateEntry> = serde_json::from_reader(f)?;
    let result: Vec<_> = result
        .into_par_iter()
        .map(FitbitHeartRate::from_json_heartrate_entry)
        .collect();
    Ok(result)
}

pub fn import_fitbit_json_files(directory: &str) -> Result<(), Error> {
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(&config.pgurl);
    for fname in glob(&format!("{}/heart_rate-*.json", directory))? {
        let fname = fname?;
        let heartrates = process_fitbit_json_file(&fname)?;
        let dates: HashSet<_> = heartrates
            .par_iter()
            .map(|entry| {
                format!(
                    "{:04}-{:02}-{:02}",
                    entry.datetime.year(),
                    entry.datetime.month(),
                    entry.datetime.day()
                )
            })
            .collect();
        let mut current_datetimes = HashSet::new();
        for date in &dates {
            for entry in FitbitHeartRate::read_from_db(&pool, &date).unwrap() {
                current_datetimes.insert(entry.datetime);
            }
        }
        println!(
            "fname {:?} {} {} {}",
            fname,
            heartrates.len(),
            dates.len(),
            current_datetimes.len()
        );
        let results: Result<Vec<_>, Error> = heartrates
            .par_iter()
            .map(|entry| {
                if !current_datetimes.contains(&entry.datetime) {
                    entry.insert_into_db(&pool.clone())?;
                }
                Ok(())
            })
            .collect();
        results?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;
    use std::collections::HashSet;
    use std::path::Path;

    // use garmin_lib::common::garmin_config::GarminConfig;
    // use garmin_lib::common::pgpool::PgPool;

    use crate::fitbit_heartrate::process_fitbit_json_file;

    #[test]
    fn test_process_fitbit_json_file() {
        // let config = GarminConfig::get_config(None).unwrap();
        // let pool = PgPool::new(&config.pgurl);
        let path = Path::new(
            "/home/ddboline/Downloads/tmp/DanielBoline/user-site-export/heart_rate-2019-01-01.json",
        );
        let result = process_fitbit_json_file(&path).unwrap();
        println!("{}", result.len());

        let dates: HashSet<_> = result
            .iter()
            .map(|entry| {
                format!(
                    "{:04}-{:02}-{:02}",
                    entry.datetime.year(),
                    entry.datetime.month(),
                    entry.datetime.day()
                )
            })
            .collect();
        assert_eq!(result.len(), 10168);
        assert_eq!(dates.len(), 2);
        // let mut current_datetimes = HashSet::new();
        // for date in dates {
        //     for entry in FitbitHeartRate::read_from_db(&pool, &date).unwrap() {
        //         current_datetimes.insert(entry.datetime);
        //     }
        // }
        // println!("{}", current_datetimes.len());
        // for entry in &result {
        //     if !current_datetimes.contains(&entry.datetime) {
        //         entry.insert_into_db(&pool).unwrap();
        //     }
        // }
    }

    // #[test]
    // fn test_import_fitbit_json_files() {
    //     import_fitbit_json_files()
    //         .unwrap();
    //     assert!(false);
    // }
}
