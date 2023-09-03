use anyhow::{format_err, Error};
use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use fitparser::{profile::field_types::MesgNum, Value};
use futures::{future::try_join_all, stream::FuturesUnordered, TryStreamExt};
use glob::glob;
use log::{debug, info};
use rayon::{
    iter::{IntoParallelIterator, ParallelExtend, ParallelIterator},
    slice::ParallelSliceMut,
};
use serde::{self, Deserialize, Deserializer, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
    fs::{rename, File},
    path::{Path, PathBuf},
};
use time::{
    macros::format_description, Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time,
};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt, PrimitiveDateTimeExt};
use tokio::task::spawn_blocking;

use garmin_connect_lib::garmin_connect_hr_data::GarminConnectHrData;
use garmin_lib::{
    common::{
        garmin_config::GarminConfig, garmin_file::GarminFile,
        garmin_summary::get_list_of_files_from_db, pgpool::PgPool,
    },
    utils::date_time_wrapper::DateTimeWrapper,
};

use crate::fitbit_statistics_summary::FitbitStatisticsSummary;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
pub struct FitbitHeartRate {
    pub datetime: DateTimeWrapper,
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
    #[serde(rename = "dateTime", deserialize_with = "deserialize_json_mdyhms")]
    pub datetime: DateTimeWrapper,
    pub value: JsonHeartRateValue,
}

/// # Errors
/// Returns error if deserialize/parse datetime fails
pub fn deserialize_json_mdyhms<'de, D>(deserializer: D) -> Result<DateTimeWrapper, D::Error>
where
    D: Deserializer<'de>,
{
    let local = DateTimeWrapper::local_tz();
    String::deserialize(deserializer).and_then(|s| {
        let d_t: SmallVec<[_; 2]> = s.split(' ').take(2).collect();
        let mdy: Result<SmallVec<[u32; 3]>, _> =
            d_t[0].split('/').take(3).map(str::parse).collect();
        let mdy = mdy.map_err(serde::de::Error::custom)?;
        let hms: Result<SmallVec<[u8; 3]>, _> = d_t[1].split(':').take(3).map(str::parse).collect();
        let hms = hms.map_err(serde::de::Error::custom)?;
        let month: Month = (mdy[0] as u8)
            .try_into()
            .map_err(serde::de::Error::custom)?;
        let day = mdy[1] as u8;
        let year = mdy[2] as i32 + 2000;
        let hour = hms[0];
        let minute = hms[1];
        let second = hms[2];

        let d = Date::from_calendar_date(year, month, day).map_err(serde::de::Error::custom)?;
        let t = Time::from_hms(hour, minute, second).map_err(serde::de::Error::custom)?;

        Ok(PrimitiveDateTime::new(d, t)
            .assume_timezone(local)
            .unwrap()
            .to_timezone(UTC)
            .into())
    })
}

impl FitbitHeartRate {
    #[must_use]
    pub fn from_json_heartrate_entry(entry: JsonHeartRateEntry) -> Self {
        Self {
            datetime: entry.datetime,
            value: entry.value.bpm,
        }
    }

    /// # Errors
    /// Returns error if api call fails
    #[allow(clippy::similar_names)]
    pub async fn get_heartrate_values(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: Date,
        end_date: Date,
    ) -> Result<Vec<(DateTimeWrapper, i32)>, Error> {
        let ndays = (end_date - start_date).whole_days();

        let days: Vec<_> = (0..=ndays)
            .map(|i| start_date + Duration::days(i))
            .collect();
        let fitbit_files: Vec<_> = days
            .iter()
            .filter_map(|date| {
                let date_str = StackString::from_display(date);
                let input_filename = config.fitbit_cachedir.join(date_str).with_extension("avro");
                if input_filename.exists() {
                    Some(input_filename)
                } else {
                    None
                }
            })
            .collect();
        info!("fitbit_files {:?}", fitbit_files);
        let futures = days.iter().map(|date| async move {
            let constraint = format_sstr!("date(begin_datetime at time zone 'utc') = '{date}'");
            let files: Vec<_> = get_list_of_files_from_db(&constraint, pool)
                .await?
                .try_filter_map(|filename| async move {
                    let avro_file = config.cache_dir.join(&format_sstr!("{filename}.avro"));
                    if avro_file.exists() {
                        Ok(Some(avro_file))
                    } else {
                        Ok(None)
                    }
                })
                .try_collect()
                .await?;
            info!("files {} {}", date, files.len());
            Ok(files)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let garmin_files: Vec<_> = results?.into_iter().flatten().collect();

        spawn_blocking(move || Self::read_fitbit_and_garmin_files(&fitbit_files, &garmin_files))
            .await?
    }

    fn read_fitbit_and_garmin_files(
        fitbit_files: &[PathBuf],
        garmin_files: &[PathBuf],
    ) -> Result<Vec<(DateTimeWrapper, i32)>, Error> {
        let results: Result<Vec<_>, Error> = fitbit_files
            .into_par_iter()
            .map(|input_path| {
                info!("read file {:?}", input_path);
                let values: Vec<_> = Self::read_avro(input_path)?
                    .into_par_iter()
                    .map(|h| (h.datetime, h.value))
                    .collect();
                info!("values {:?} {}", input_path, values.len());
                Ok(values)
            })
            .collect();
        let mut heartrate_values: Vec<_> = results?.into_par_iter().flatten().collect();

        let results: Result<Vec<_>, Error> = garmin_files
            .into_par_iter()
            .map(|avro_file| {
                let points: Vec<_> = GarminFile::read_avro(avro_file)?
                    .points
                    .into_par_iter()
                    .filter_map(|p| p.heart_rate.map(|h| (p.time, h as i32)))
                    .collect();
                Ok(points)
            })
            .collect();
        heartrate_values.par_extend(results?.into_par_iter().flatten());
        heartrate_values.par_sort();
        heartrate_values.dedup();
        info!("heartrate_values {}", heartrate_values.len());
        Ok(heartrate_values)
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn calculate_summary_statistics(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: Date,
    ) -> Result<Option<FitbitStatisticsSummary>, Error> {
        let heartrate_values =
            Self::get_heartrate_values(config, pool, start_date, start_date).await?;

        if let Some(hr_val) = FitbitStatisticsSummary::from_heartrate_values(&heartrate_values) {
            hr_val.upsert_entry(pool).await?;
            Ok(Some(hr_val))
        } else {
            Ok(None)
        }
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_all_summary_statistics(
        config: &GarminConfig,
        pool: &PgPool,
    ) -> Result<(), Error> {
        let dates: Result<Vec<_>, Error> = glob(&format_sstr!(
            "{}/*.avro",
            config.fitbit_cachedir.to_string_lossy()
        ))?
        .map(|x| {
            x.map_err(Into::into).and_then(|f| {
                let date = Date::parse(
                    &f.file_stem()
                        .ok_or_else(|| format_err!("No name"))?
                        .to_string_lossy(),
                    format_description!("[year]-[month]-[day]"),
                )?;
                Ok(date)
            })
        })
        .collect();
        let dates = dates?;
        let futures: FuturesUnordered<_> = dates
            .into_iter()
            .map(|date| {
                let config = config.clone();
                let pool = pool.clone();
                async move {
                    Self::calculate_summary_statistics(&config, &pool, date).await?;
                    debug!("{}", date);
                    Ok(())
                }
            })
            .collect();
        futures.try_collect().await
    }

    /// # Errors
    /// Returns error if serialize to avro file fails
    pub fn dump_to_avro(values: &[Self], output_filename: impl AsRef<Path>) -> Result<(), Error> {
        use rand::{
            distributions::{Alphanumeric, DistString},
            thread_rng,
        };
        let schema = Schema::parse_str(FITBITHEARTRATE_SCHEMA)?;

        let tmp_path = {
            let mut rng = thread_rng();
            let rand_str = Alphanumeric.sample_string(&mut rng, 8);
            output_filename
                .as_ref()
                .with_file_name(format_sstr!(".tmp_{rand_str}"))
        };

        let output_file = File::create(&tmp_path)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);
        writer.append_ser(values)?;
        writer.flush()?;

        rename(&tmp_path, output_filename)?;
        Ok(())
    }

    /// # Errors
    /// Returns error if `read_avro` fails
    pub fn read_avro_by_date(config: &GarminConfig, date: Date) -> Result<Vec<Self>, Error> {
        let date_str = StackString::from_display(date);
        let input_filename = config.fitbit_cachedir.join(date_str).with_extension("avro");
        debug!("avro {:?}", input_filename);
        if input_filename.exists() {
            Self::read_avro(&input_filename)
        } else {
            Ok(Vec::new())
        }
    }

    /// # Errors
    /// Returns error if file read fails
    pub fn read_avro(input_filename: impl AsRef<Path>) -> Result<Vec<Self>, Error> {
        if !input_filename.as_ref().exists() {
            return Err(format_err!(
                "file {:?} does not exist",
                input_filename.as_ref()
            ));
        }
        let input_file = File::open(input_filename)?;
        Reader::new(input_file)?
            .next()
            .map(|record| from_value::<Vec<Self>>(&record?))
            .transpose()
            .map(Option::unwrap_or_default)
            .map_err(Into::into)
    }

    /// # Errors
    /// Returns error if `read_avro_by_date` fails
    pub fn merge_slice_to_avro(
        config: &GarminConfig,
        values: &[Self],
    ) -> Result<BTreeSet<Date>, Error> {
        let dates: BTreeSet<_> = values
            .iter()
            .map(|entry| entry.datetime.to_timezone(UTC).date())
            .collect();
        let mut output = Vec::new();
        for date in &dates {
            let date = *date;
            let new_values = values.iter().filter_map(|entry| {
                if entry.datetime.to_timezone(UTC).date() == date {
                    Some(*entry)
                } else {
                    None
                }
            });
            let mut merged_values: Vec<_> = Self::read_avro_by_date(config, date)?
                .into_iter()
                .chain(new_values)
                .filter(|h| h.value > 0)
                .collect();
            merged_values.par_sort_by_key(|entry| entry.datetime.unix_timestamp());
            merged_values.dedup();
            let date_str = StackString::from_display(date);
            let input_filename = config.fitbit_cachedir.join(date_str).with_extension("avro");
            Self::dump_to_avro(&merged_values, &input_filename)?;
            output.push(date);
        }
        Ok(dates)
    }

    pub fn from_garmin_connect_hr(hr_data: &GarminConnectHrData) -> Vec<Self> {
        hr_data
            .heartrate_values
            .as_ref()
            .map_or_else(Vec::new, |hr_vals| {
                hr_vals
                    .iter()
                    .filter_map(|(timestamp, hr_val_opt)| {
                        hr_val_opt.map(|value| {
                            let datetime: OffsetDateTime = (*timestamp).into();
                            let datetime = datetime.into();
                            Self { datetime, value }
                        })
                    })
                    .collect()
            })
    }
}

/// # Errors
/// Returns error if deserialization fails
pub fn process_fitbit_json_file(fname: &Path) -> Result<Vec<FitbitHeartRate>, Error> {
    if !fname.exists() {
        return Err(format_err!("file {fname:?} does not exist"));
    }
    let f = File::open(fname)?;
    let result: Vec<JsonHeartRateEntry> = serde_json::from_reader(f)?;
    let result: Vec<_> = result
        .into_par_iter()
        .map(FitbitHeartRate::from_json_heartrate_entry)
        .collect();
    Ok(result)
}

/// # Errors
/// Returns error if deserialization fails
pub fn import_fitbit_json_files(
    config: &GarminConfig,
    directory: &str,
) -> Result<BTreeSet<Date>, Error> {
    let filenames: Vec<_> = glob(&format_sstr!("{directory}/heart_rate-*.json"))?.collect();
    let result: Result<Vec<BTreeSet<Date>>, Error> = filenames
        .into_par_iter()
        .map(|fname| {
            let fname = fname?;
            let heartrates = process_fitbit_json_file(&fname)?;

            FitbitHeartRate::merge_slice_to_avro(config, &heartrates)
        })
        .collect();
    result.map(|v| v.into_iter().flatten().collect())
}

/// # Errors
/// Returns error if deserialization fails
pub fn import_garmin_json_file(config: &GarminConfig, filename: &Path) -> Result<(), Error> {
    if !filename.exists() {
        return Err(format_err!("file {filename:?} does not exist"));
    }
    let js: GarminConnectHrData = serde_json::from_reader(File::open(filename)?)?;

    let heartrates = FitbitHeartRate::from_garmin_connect_hr(&js);

    FitbitHeartRate::merge_slice_to_avro(config, &heartrates)?;

    Ok(())
}

/// # Errors
/// Returns error if deserialization fails
pub fn import_garmin_heartrate_file(
    config: &GarminConfig,
    filename: &Path,
) -> Result<BTreeSet<Date>, Error> {
    let mut timestamps = Vec::new();
    let mut heartrates = Vec::new();
    if !filename.exists() {
        return Err(format_err!("file {filename:?} does not exist"));
    }
    let mut f = File::open(filename)?;
    let records = fitparser::from_reader(&mut f).map_err(|e| format_err!("{e:?}"))?;
    for record in &records {
        match record.kind() {
            MesgNum::Monitoring => {
                let mut timestamp: Option<_> = None;
                let mut timestamp_16: Option<_> = None;
                let mut heartrate: Option<u8> = None;
                for field in record.fields() {
                    match field.name() {
                        "timestamp" => {
                            info!("timestamp {:?}", field.value());
                            if let Value::Timestamp(t) = field.value() {
                                timestamp.replace(*t);
                            }
                        }
                        "timestamp_16" => {
                            info!("timestamp_16 {:?}", field.value());
                            if let Value::UInt16(v) = field.value() {
                                timestamp_16.replace(*v);
                            }
                        }
                        "heart_rate" => {
                            info!("heartrate {:?}", field.value());
                            if let Value::UInt8(v) = field.value() {
                                if *v > 0 {
                                    heartrate.replace(*v);
                                }
                            }
                        }
                        _ => {
                            info!("fieldname {} {:?}", field.name(), field.value());
                        }
                    }
                    if let Some(t) = timestamp {
                        timestamps.push(t);
                    }
                    if let Some(t16) = timestamp_16 {
                        if let Some(h) = heartrate {
                            heartrates.push((t16, h));
                        }
                    }
                }
            }
            other => info!("other {other:?}"),
        }
    }

    if timestamps.is_empty() || heartrates.is_empty() {
        return Ok(BTreeSet::new());
    }

    let min_timestamp = *timestamps.first().expect("No timestamps");
    let max_timestamp = *timestamps.iter().last().expect("No timestamps");
    let min_timestamp16 = i64::from(heartrates.first().expect("No heartrates").0);
    let max_timestamp16 = i64::from(heartrates.iter().last().expect("No heartrates").0);

    info!(
        "timestamps {} {} heartrates {} {}",
        min_timestamp, max_timestamp, min_timestamp16, max_timestamp16
    );

    let heartrates: BTreeMap<i64, FitbitHeartRate> = heartrates
        .iter()
        .map(|(t16, h)| {
            let t16 = i64::from(*t16);
            let mut diff = t16 - min_timestamp16;
            if diff < 0 {
                diff += i64::from(u16::MAX);
            }
            let datetime: DateTimeWrapper = (min_timestamp + Duration::seconds(diff)).into();
            let value = i32::from(*h);
            let key = datetime.unix_timestamp();
            (key, FitbitHeartRate { datetime, value })
        })
        .collect();

    let heartrates: Vec<FitbitHeartRate> = heartrates.into_values().collect();

    FitbitHeartRate::merge_slice_to_avro(config, &heartrates)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FitbitBodyWeightFat {
    pub datetime: DateTimeWrapper,
    pub weight: f64,
    pub fat: f64,
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use log::debug;
    use std::{collections::HashSet, path::Path};
    use time::macros::date;
    use time_tz::OffsetDateTimeExt;

    use garmin_lib::{
        common::{garmin_config::GarminConfig, pgpool::PgPool},
        utils::date_time_wrapper::DateTimeWrapper,
    };

    use crate::fitbit_heartrate::{process_fitbit_json_file, FitbitHeartRate};

    #[test]
    #[ignore]
    fn test_process_fitbit_json_file() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let path = Path::new("tests/data/test_heartrate_data.json");
        let result = process_fitbit_json_file(&path)?;
        debug!("{}", result.len());
        let local = DateTimeWrapper::local_tz();
        let dates: HashSet<_> = result
            .iter()
            .map(|entry| entry.datetime.to_timezone(local).date())
            .collect();
        debug!("{:?}", dates);
        let dates = vec![date!(2019 - 11 - 01)];
        assert_eq!(result.len(), 3);
        assert_eq!(dates.len(), 1);

        let mut current_datetimes = HashSet::new();
        for date in dates {
            for entry in FitbitHeartRate::read_avro_by_date(&config, date)? {
                current_datetimes.insert(entry.datetime);
            }
        }
        debug!("{}", current_datetimes.len());
        assert_eq!(current_datetimes.len(), 11212);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate_summary_statistics() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let start_date = date!(2019 - 08 - 01);
        let result =
            FitbitHeartRate::calculate_summary_statistics(&config, &pool, start_date).await?;
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.min_heartrate as i32, 39);
        assert_eq!(result.max_heartrate as i32, 181);
        assert_eq!(result.median_heartrate as i32, 62);
        assert_eq!(result.number_of_entries as i32, 12597);
        Ok(())
    }
}
