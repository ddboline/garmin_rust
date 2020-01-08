use anyhow::{format_err, Error};
use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use chrono::{
    DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use cpython::{exc, FromPyObject, PyDict, PyErr, PyResult, Python};
use glob::glob;
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{self, Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::Path;
use structopt::StructOpt;

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::common::garmin_summary::get_list_of_files_from_db;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::reports::garmin_templates::{PLOT_TEMPLATE, TIMESERIESTEMPLATE};
use garmin_lib::utils::iso_8601_datetime;

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

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct FitbitHeartRate {
    #[serde(with = "iso_8601_datetime")]
    pub datetime: DateTime<Utc>,
    pub value: i32,
}

pub const FITBITHEARTRATE_SCHEMA: &str = r#"
    {
        "namespace": "fitbit.avro",
        "type": "array",
        "items": {
            "namespace": "fitbit.avro",
            "type": "record",
            "name": "FitbitHeartRatePoint",
            "fields": [
                {"name": "datetime", "type": "string"},
                {"name": "value", "type": "int"}
            ]
        }
    }
"#;

#[derive(Deserialize, Copy, Clone)]
pub struct JsonHeartRateValue {
    pub bpm: i32,
    pub confidence: i32,
}

#[derive(Deserialize, Copy, Clone)]
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

    pub fn from_json_heartrate_entry(entry: JsonHeartRateEntry) -> Self {
        Self {
            datetime: entry.datetime,
            value: entry.value.bpm,
        }
    }

    pub fn get_heartrate_plot(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<String, Error> {
        let nminutes = 5;
        let ndays = (end_date - start_date).num_days();
        let heartrate_values: Result<Vec<_>, Error> = (0..=ndays)
            .into_par_iter()
            .map(|i| {
                let mut heartrate_values = Vec::new();
                let date = start_date + Duration::days(i);
                let input_filename = format!("{}/{}.avro", config.fitbit_cachedir, date);
                let values: Vec<_> = Self::read_avro(&input_filename)
                    .unwrap_or_else(|_| Vec::new())
                    .into_iter()
                    .map(|h| (h.datetime, h.value))
                    .collect();
                heartrate_values.extend_from_slice(&values);
                let constraint = format!("date(begin_datetime at time zone 'utc') = '{}'", date);
                for filename in get_list_of_files_from_db(&[constraint], pool)? {
                    let avro_file = format!("{}/{}.avro", &config.cache_dir, filename);
                    let points: Vec<_> = GarminFile::read_avro(&avro_file)?
                        .points
                        .into_iter()
                        .filter_map(|p| p.heart_rate.map(|h| (p.time, h as i32)))
                        .collect();
                    heartrate_values.extend_from_slice(&points);
                }
                Ok(heartrate_values)
            })
            .collect();
        let heartrate_values: Vec<_> = heartrate_values?.into_iter().flatten().collect();
        let mut final_values = Vec::new();
        for (_, group) in &heartrate_values
            .into_iter()
            .group_by(|(d, _)| d.timestamp() / (i64::from(nminutes) * 60))
        {
            let g: Vec<_> = group.collect();
            let d = g.iter().map(|(d, _)| *d).min();
            if let Some(d) = d {
                let v = g.iter().map(|(_, v)| v).sum::<i32>() / g.len() as i32;
                let d = d.format("%Y-%m-%dT%H:%M:%S%z").to_string();
                final_values.push((d, v));
            }
        }
        final_values.sort();
        let js_str = serde_json::to_string(&final_values).unwrap_or_else(|_| "".to_string());
        let plots = TIMESERIESTEMPLATE
            .replace("DATA", &js_str)
            .replace("EXAMPLETITLE", "Heart Rate")
            .replace("XAXIS", "Date")
            .replace("YAXIS", "Heart Rate");
        let plots = format!("<script>\n{}\n</script>", plots);
        let buttons: Vec<_> = (0..10)
            .map(|i| {
                let date = Local::today().naive_local() - Duration::days(i);
                format!(
                    r#"
            <button type="submit" id="ID"
             onclick="heartrate_plot_date('{date}','{date}');"">Plot {date}</button>
            <button type="submit" id="ID"
             onclick="heartrate_sync('{date}');">Sync {date}</button><br>"#,
                    date = date
                )
            })
            .collect();
        let body = PLOT_TEMPLATE
            .replace("INSERTOTHERIMAGESHERE", &plots)
            .replace("INSERTTEXTHERE", &buttons.join("\n"));
        Ok(body)
    }

    pub fn dump_to_avro(values: &[Self], output_filename: &str) -> Result<(), Error> {
        let schema = Schema::parse_str(FITBITHEARTRATE_SCHEMA).map_err(|e| format_err!("{}", e))?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer
            .append_ser(values)
            .and_then(|_| writer.flush().map(|_| ()))
            .map_err(|e| format_err!("{}", e))
    }

    pub fn read_avro_by_date(config: &GarminConfig, date: NaiveDate) -> Result<Vec<Self>, Error> {
        let input_filename = format!("{}/{}.avro", config.fitbit_cachedir, date);
        debug!("avro {}", input_filename);
        if Path::new(&input_filename).exists() {
            Self::read_avro(&input_filename)
        } else {
            Ok(Vec::new())
        }
    }

    pub fn read_avro(input_filename: &str) -> Result<Vec<Self>, Error> {
        let input_file = File::open(input_filename)?;

        Reader::new(input_file)
            .map_err(|e| format_err!("{}", e))?
            .nth(0)
            .map(|record| {
                let record = record.map_err(|e| format_err!("{}", e))?;
                from_value::<Vec<Self>>(&record).map_err(|e| format_err!("{}", e))
            })
            .transpose()
            .map(|x| x.unwrap_or_else(Vec::new))
            .map_err(|e| format_err!("{}", e))
    }

    pub fn merge_slice_to_avro(config: &GarminConfig, values: &[Self]) -> Result<(), Error> {
        let dates: HashSet<_> = values
            .par_iter()
            .map(|entry| entry.datetime.naive_utc().date())
            .collect();
        for date in dates {
            let new_values = values.par_iter().filter_map(|entry| {
                if entry.datetime.naive_utc().date() == date {
                    Some(*entry)
                } else {
                    None
                }
            });
            let merged_values: BTreeMap<_, _> = Self::read_avro_by_date(config, date)?
                .into_par_iter()
                .chain(new_values)
                .map(|entry| (entry.datetime.timestamp(), entry))
                .collect();
            let input_filename = format!("{}/{}.avro", config.fitbit_cachedir, date);
            let merged_values: Vec<_> = merged_values.values().copied().collect();
            Self::dump_to_avro(&merged_values, &input_filename)?;
        }
        Ok(())
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

#[derive(StructOpt, Debug, Clone)]
pub struct JsonImportOpts {
    #[structopt(short = "d", long = "directory")]
    pub directory: String,
}

pub fn import_fitbit_json_files(directory: &str) -> Result<(), Error> {
    let config = GarminConfig::get_config(None)?;
    let filenames: Vec<_> = glob(&format!("{}/heart_rate-*.json", directory))?.collect();
    filenames
        .into_par_iter()
        .map(|fname| {
            let fname = fname?;
            let heartrates = process_fitbit_json_file(&fname)?;

            FitbitHeartRate::merge_slice_to_avro(&config, &heartrates)
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
pub struct FitbitBodyWeightFat {
    pub datetime: DateTime<Utc>,
    pub weight: f64,
    pub fat: f64,
}

impl FitbitBodyWeightFat {
    pub fn from_pydict(
        py: Python,
        dict: PyDict,
        offset: FixedOffset,
    ) -> PyResult<FitbitBodyWeightFat> {
        let date: NaiveDate = get_pydict_item!(py, dict, date, String)?.parse().unwrap();
        let time: NaiveTime = get_pydict_item!(py, dict, time, String)?.parse().unwrap();
        let datetime = format!("{}T{}{}", date, time, offset);
        let datetime = DateTime::parse_from_rfc3339(&datetime)
            .unwrap()
            .with_timezone(&Utc);
        let weight = get_pydict_item!(py, dict, weight, f64)?;
        let fat = get_pydict_item!(py, dict, fat, f64)?;
        Ok(FitbitBodyWeightFat {
            datetime,
            weight,
            fat,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use std::collections::HashSet;
    use std::io::{stdout, Write};
    use std::path::Path;

    use garmin_lib::common::garmin_config::GarminConfig;
    use garmin_lib::common::pgpool::PgPool;

    use crate::fitbit_heartrate::{process_fitbit_json_file, FitbitHeartRate};

    #[test]
    #[ignore]
    fn test_process_fitbit_json_file() {
        let config = GarminConfig::get_config(None).unwrap();
        let path = Path::new("tests/data/test_heartrate_data.json");
        let result = process_fitbit_json_file(&path).unwrap();
        writeln!(stdout(), "{}", result.len()).unwrap();

        let dates: HashSet<_> = result
            .iter()
            .map(|entry| entry.datetime.date().naive_local())
            .collect();
        writeln!(stdout(), "{:?}", dates).unwrap();
        let dates = vec![NaiveDate::from_ymd(2019, 11, 1)];
        assert_eq!(result.len(), 3);
        assert_eq!(dates.len(), 1);

        let mut current_datetimes = HashSet::new();
        for date in dates {
            for entry in FitbitHeartRate::read_avro_by_date(&config, date).unwrap() {
                current_datetimes.insert(entry.datetime);
            }
        }
        writeln!(stdout(), "{}", current_datetimes.len()).unwrap();
        assert_eq!(current_datetimes.len(), 1361);
    }

    #[test]
    #[ignore]
    fn test_get_heartrate_plot() {
        let config = GarminConfig::get_config(None).unwrap();
        let pool = PgPool::new(&config.pgurl);
        let start_date = NaiveDate::from_ymd(2019, 8, 1);
        let end_date = NaiveDate::from_ymd(2019, 8, 2);
        let results =
            FitbitHeartRate::get_heartrate_plot(&config, &pool, start_date, end_date).unwrap();
        writeln!(stdout(), "{}", results).unwrap();
        assert!(results.len() > 0);
    }

    // #[test]
    // fn test_import_fitbit_json_files() {
    //     import_fitbit_json_files()
    //         .unwrap();
    //     assert!(false);
    // }
}
