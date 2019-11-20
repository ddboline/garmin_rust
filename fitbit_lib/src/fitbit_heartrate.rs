use chrono::{
    DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use cpython::{exc, FromPyObject, PyDict, PyErr, PyResult, Python};
use failure::{err_msg, Error};
use glob::glob;
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{self, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use structopt::StructOpt;

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::common::garmin_summary::get_list_of_files_from_db;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::reports::garmin_templates::{PLOT_TEMPLATE, TIMESERIESTEMPLATE};
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

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct FitbitHeartRate {
    pub datetime: DateTime<Utc>,
    pub value: i32,
}

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

    pub fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let conn = pool.get()?;
        let query = "
            INSERT INTO fitbit_heartrate (datetime, bpm)
            SELECT $1, $2
            WHERE NOT EXISTS
            (SELECT datetime FROM fitbit_heartrate WHERE datetime = $1)";
        conn.execute(query, &[&self.datetime, &self.value])
            .map(|_| ())
            .map_err(err_msg)
    }

    pub fn from_json_heartrate_entry(entry: JsonHeartRateEntry) -> Self {
        Self {
            datetime: entry.datetime,
            value: entry.value.bpm,
        }
    }

    pub fn read_from_db(pool: &PgPool, date: NaiveDate) -> Result<Vec<Self>, Error> {
        let query = "
            SELECT datetime, bpm
            FROM fitbit_heartrate
            WHERE date(datetime) = $1
            ORDER BY datetime";
        let conn = pool.get()?;
        conn.query(&query, &[&date])?
            .iter()
            .map(|row| {
                let datetime = row.get_idx(0)?;
                let value = row.get_idx(1)?;
                Ok(Self { datetime, value })
            })
            .collect()
    }

    pub fn read_count_from_db(
        pool: &PgPool,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<(NaiveDate, i64)>, Error> {
        let query = "
            SELECT date(datetime), count(*) FROM fitbit_heartrate
            WHERE date(datetime) >= $1 AND date(datetime) <= $2
            GROUP BY 1 ORDER BY 1";
        let conn = pool.get()?;
        conn.query(&query, &[&start_date, &end_date])?
            .iter()
            .map(|row| {
                let date: NaiveDate = row.get_idx(0)?;
                let count: i64 = row.get_idx(1)?;
                Ok((date, count))
            })
            .collect()
    }

    pub fn insert_slice_into_db(slice: &[Self], pool: &PgPool) -> Result<(), Error> {
        let conn = pool.get()?;
        let trans = conn.transaction()?;
        let query = "
            CREATE TEMP TABLE fitbit_heartrate_temp
            AS (SELECT datetime, bpm FROM fitbit_heartrate limit 0)";
        trans.execute(query, &[])?;
        let query = "
            INSERT INTO fitbit_heartrate_temp (datetime, bpm)
            SELECT $1, $2";
        let stmt = trans.prepare(query)?;
        let results: Result<_, Error> = slice
            .iter()
            .map(|entry| {
                stmt.execute(&[&entry.datetime, &entry.value])
                    .map_err(err_msg)
                    .map(|_| ())
            })
            .collect();
        results?;
        let query = "
            INSERT INTO fitbit_heartrate (datetime, bpm)
            SELECT a.datetime, a.bpm
            FROM fitbit_heartrate_temp a
            LEFT JOIN fitbit_heartrate b
                ON cast(extract(epoch from a.datetime) as int) = cast(extract(epoch from b.datetime) as int)
            WHERE b.datetime is NULL";
        trans.execute(query, &[])?;
        let query = "DROP TABLE fitbit_heartrate_temp";
        trans.execute(query, &[])?;
        trans.commit()?;
        Ok(())
    }

    pub fn read_from_db_resample(
        pool: &PgPool,
        date: NaiveDate,
        nminutes: usize,
    ) -> Result<Vec<Self>, Error> {
        let query = format!(
            "
            SELECT min(datetime), cast(avg(bpm) as int)
            FROM fitbit_heartrate
            WHERE date(datetime) = $1
            GROUP BY cast(extract(epoch from datetime)/({}*60) as int)
            ORDER BY 1",
            nminutes
        );
        debug!("{}", &query);
        let conn = pool.get()?;
        conn.query(&query, &[&date])?
            .iter()
            .map(|row| {
                let datetime = row.get_idx(0)?;
                let value = row.get_idx(1)?;
                Ok(Self { datetime, value })
            })
            .collect()
    }

    pub fn get_heartrate_plot(
        pool: &PgPool,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<String, Error> {
        let nminutes = 5;
        let config = GarminConfig::get_config(None)?;
        let ndays = (end_date - start_date).num_days();
        let heartrate_values: Result<Vec<_>, Error> = (0..=ndays)
            .into_par_iter()
            .map(|i| {
                let mut heartrate_values = Vec::new();
                let date = start_date + Duration::days(i);
                let values: Vec<_> = FitbitHeartRate::read_from_db_resample(pool, date, nminutes)?
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
            .group_by(|(d, _)| d.timestamp() / (nminutes as i64 * 60))
        {
            let g: Vec<_> = group.collect();
            let d = g.iter().map(|(d, _)| *d).min();
            if let Some(d) = d {
                let v = g.iter().map(|(_, v)| v).sum::<i32>() / g.len() as i32;
                let d = d.format("%Y-%m-%dT%H:%M:%SZ").to_string();
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
    let pool = PgPool::new(&config.pgurl);
    let filenames: Vec<_> = glob(&format!("{}/heart_rate-*.json", directory))?.collect();
    filenames
        .into_par_iter()
        .map(|fname| {
            let fname = fname?;
            let heartrates = process_fitbit_json_file(&fname)?;
            let dates: HashSet<_> = heartrates
                .par_iter()
                .map(|entry| entry.datetime.date().naive_local())
                .collect();
            let mut current_datetimes = HashSet::new();
            for date in &dates {
                for entry in FitbitHeartRate::read_from_db(&pool, *date).unwrap() {
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
            heartrates
                .par_iter()
                .map(|entry| {
                    if !current_datetimes.contains(&entry.datetime) {
                        entry.insert_into_db(&pool.clone())?;
                    }
                    Ok(())
                })
                .collect::<Result<(), Error>>()
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
    use std::collections::HashSet;
    use std::path::Path;

    use garmin_lib::common::garmin_config::GarminConfig;
    use garmin_lib::common::pgpool::PgPool;

    use crate::fitbit_heartrate::{process_fitbit_json_file, FitbitHeartRate};

    #[test]
    fn test_process_fitbit_json_file() {
        let config = GarminConfig::get_config(None).unwrap();
        let pool = PgPool::new(&config.pgurl);
        let path = Path::new("tests/data/test_heartrate_data.json");
        let result = process_fitbit_json_file(&path).unwrap();
        println!("{}", result.len());

        let dates: HashSet<_> = result
            .iter()
            .map(|entry| entry.datetime.date().naive_local())
            .collect();
        assert_eq!(result.len(), 3);
        assert_eq!(dates.len(), 1);
        let mut current_datetimes = HashSet::new();
        for date in dates {
            for entry in FitbitHeartRate::read_from_db(&pool, date).unwrap() {
                current_datetimes.insert(entry.datetime);
            }
        }
        println!("{}", current_datetimes.len());
        assert_eq!(current_datetimes.len(), 10232);
    }

    // #[test]
    // fn test_import_fitbit_json_files() {
    //     import_fitbit_json_files()
    //         .unwrap();
    //     assert!(false);
    // }
}
