use anyhow::Error;
use futures::{future::try_join_all, TryStreamExt};
use log::info;
use polars::{
    frame::DataFrame,
    prelude::{
        Int32Chunked, Int64Chunked, IntoSeries, NewChunkedArray, ParquetReader, ParquetWriter,
        SerReader, UniqueKeepStrategy,
    },
};
use stack_string::{format_sstr, StackString};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
    fs::File,
    path::PathBuf,
};
use time::{Date, Duration, Month, OffsetDateTime, Time};
use tokio::task::spawn_blocking;

use garmin_lib::{
    common::{
        garmin_config::GarminConfig, garmin_file::GarminFile,
        garmin_summary::get_list_of_files_from_db, pgpool::PgPool,
    },
    utils::date_time_wrapper::DateTimeWrapper,
};

use crate::fitbit_heartrate::FitbitHeartRate;

#[derive(Default)]
struct FitbitColumns {
    timestamp: Vec<i64>,
    value: Vec<i32>,
}

fn get_fitbit_avro_file_map(
    config: &GarminConfig,
    all: bool,
) -> Result<BTreeMap<StackString, BTreeSet<PathBuf>>, Error> {
    let min_date = if all {
        None
    } else {
        let d = (OffsetDateTime::now_utc() - Duration::days(60)).date();
        Some(format_sstr!("{d}"))
    };
    let mut input_files: BTreeMap<StackString, BTreeSet<_>> = BTreeMap::new();
    for p in config.fitbit_cachedir.read_dir()? {
        let p = p?.path();
        if let Some(file_name) = p.file_name() {
            let file_name = file_name.to_string_lossy();
            if let Some(date) = file_name.split(".avro").next() {
                let key = format_sstr!("{}", &date[0..7]);
                if let Some(min_date) = &min_date {
                    let file = config
                        .fitbit_archivedir
                        .join(&format_sstr!("{key}.parquet"));
                    if file.exists() && date < min_date.as_str() {
                        continue;
                    }
                }
                input_files.entry(key).or_default().insert(p);
            }
        }
    }
    Ok(input_files)
}

async fn get_garmin_avro_file_map(
    config: &GarminConfig,
    pool: &PgPool,
    start_date: Date,
    end_date: Date,
) -> Result<Vec<PathBuf>, Error> {
    let ndays = (end_date - start_date).whole_days();
    let days: Vec<_> = (0..=ndays)
        .map(|i| start_date + Duration::days(i))
        .collect();
    let futures = days.iter().map(|date| async move {
        let constraint = format_sstr!(
            r#"
                date(begin_datetime at time zone 'utc') >= '{date}' AND
                date(begin_datetime at time zone 'utc' + ('1 second'::interval * total_duration)) <= '{date}'
            "#);
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
    Ok(garmin_files)
}

fn get_start_date_end_date_for_key(key: &str) -> Result<(Date, Date), Error> {
    let year: i32 = (&key[0..4]).parse()?;
    let month: u8 = (&key[5..7]).parse()?;
    let month: Month = month.try_into()?;
    let start_date = Date::from_calendar_date(year, month, 1)?;
    let mut end_date = start_date;
    while let Some(next_date) = end_date.next_day() {
        if next_date.month() != month {
            break;
        } else {
            end_date = next_date
        }
    }
    Ok((start_date, end_date))
}

pub async fn archive_fitbit_heartrates(
    config: &GarminConfig,
    pool: &PgPool,
    all: bool,
) -> Result<Vec<StackString>, Error> {
    let mut output = Vec::new();
    let input_files = {
        let config = config.clone();
        spawn_blocking(move || get_fitbit_avro_file_map(&config, all)).await??
    };
    let mut garmin_input_files: BTreeMap<StackString, BTreeSet<_>> = BTreeMap::new();
    for key in input_files.keys() {
        let (start_date, end_date) = get_start_date_end_date_for_key(key)?;
        garmin_input_files
            .entry(key.clone())
            .or_default()
            .extend(get_garmin_avro_file_map(config, pool, start_date, end_date).await?);
    }
    output.extend({
        let config = config.clone();
        spawn_blocking(move || {
            write_fitbit_heartrate_parquet(&config, &input_files, &garmin_input_files)
        })
        .await??
    });
    Ok(output)
}

fn write_fitbit_heartrate_parquet(
    config: &GarminConfig,
    fitbit_input_files: &BTreeMap<StackString, BTreeSet<PathBuf>>,
    garmin_input_files: &BTreeMap<StackString, BTreeSet<PathBuf>>,
) -> Result<Vec<StackString>, Error> {
    let mut output = Vec::new();
    for (key, input_files) in fitbit_input_files {
        let (start_date, end_date) = get_start_date_end_date_for_key(key)?;
        let mut heartrates: BTreeMap<i64, Vec<_>> = BTreeMap::new();
        for input_file in input_files {
            for value in FitbitHeartRate::read_avro(&input_file)? {
                let d = value.datetime.date();
                if d < start_date || d > end_date {
                    continue;
                }
                let timestamp = value.datetime.unix_timestamp();
                heartrates.entry(timestamp).or_default().push(value.value);
            }
        }
        if let Some(garmin_files) = garmin_input_files.get(key) {
            for input_file in garmin_files {
                for (t, h) in GarminFile::read_avro(&input_file)?
                    .points
                    .into_iter()
                    .filter_map(|p| p.heart_rate.map(|h| (p.time, h as i32)))
                {
                    let d = t.date();
                    if d < start_date || d > end_date {
                        continue;
                    }
                    let timestamp = t.unix_timestamp();
                    heartrates.entry(timestamp).or_default().push(h);
                }
            }
        }
        let columns = FitbitColumns {
            timestamp: heartrates.keys().map(|h| *h).collect(),
            value: heartrates
                .values()
                .map(|h| {
                    if h.len() == 1 {
                        h[0]
                    } else {
                        let s: i32 = h.iter().sum();
                        s / h.len() as i32
                    }
                })
                .collect(),
        };
        let columns = vec![
            Int64Chunked::from_slice("timestamp", &columns.timestamp).into_series(),
            Int32Chunked::from_slice("value", &columns.value).into_series(),
        ];
        let new_df = DataFrame::new(columns)?;
        let filename = format_sstr!("{key}.parquet");
        let file = config.fitbit_archivedir.join(&filename);
        let mut df = if file.exists() {
            let df = ParquetReader::new(File::open(&file)?).finish()?;
            df.vstack(&new_df)?
                .unique(None, UniqueKeepStrategy::First, None)?
        } else {
            new_df
        };
        output.push(format_sstr!("{filename} {:?}", df.shape()));
        ParquetWriter::new(File::create(&file)?).finish(&mut df)?;
    }
    Ok(output)
}

fn get_fitbit_parquet_files(
    config: &GarminConfig,
    start_date: Date,
    end_date: Date,
) -> Result<Vec<PathBuf>, Error> {
    let ndays = (end_date - start_date).whole_days();
    let keys: BTreeSet<_> = (0..=ndays)
        .map(|i| {
            let d = start_date + Duration::days(i);
            let m: u8 = d.month().into();
            format_sstr!("{:04}-{:02}", d.year(), m)
        })
        .collect();
    let fitbit_files: Vec<_> = keys
        .iter()
        .filter_map(|key| {
            let input_filename = config.fitbit_archivedir.join(key).with_extension("parquet");
            if input_filename.exists() {
                Some(input_filename)
            } else {
                None
            }
        })
        .collect();
    info!("fitbit_files {:?}", fitbit_files);
    Ok(fitbit_files)
}

pub fn get_number_of_heartrate_values(
    config: &GarminConfig,
    start_date: Date,
    end_date: Date,
) -> Result<usize, Error> {
    let fitbit_files = get_fitbit_parquet_files(config, start_date, end_date)?;
    let mut value_count = 0;
    for file in fitbit_files {
        let df = ParquetReader::new(File::open(file)?).finish()?;
        value_count += df.shape().0;
    }
    Ok(value_count)
}

pub fn get_heartrate_values(
    config: &GarminConfig,
    start_date: Date,
    end_date: Date,
    step_size: Option<u64>,
) -> Result<Vec<(DateTimeWrapper, i32)>, Error> {
    let step_size = step_size.unwrap_or(1) as i64;
    let fitbit_files = get_fitbit_parquet_files(config, start_date, end_date)?;
    let start_timestamp = start_date
        .with_time(Time::from_hms(0, 0, 0)?)
        .assume_utc()
        .unix_timestamp();
    let end_timestamp = end_date
        .with_time(Time::from_hms(23, 59, 59)?)
        .assume_utc()
        .unix_timestamp();
    let mut values: BTreeMap<i64, Vec<i32>> = BTreeMap::new();
    for file in &fitbit_files {
        let df = ParquetReader::new(File::open(file)?).finish()?;
        let timestamp_iter = df.column("timestamp")?.i64()?.into_iter();
        let value_iter = df.column("value")?.i32()?.into_iter();

        for (t, v) in timestamp_iter
            .zip(value_iter)
            .filter_map(|(timestamp, value)| {
                timestamp.and_then(|t| {
                    value.and_then(|v| {
                        let t = if step_size == 1 {
                            t
                        } else {
                            (t / step_size) * step_size
                        };
                        if t >= start_timestamp && t <= end_timestamp {
                            Some((t, v))
                        } else {
                            None
                        }
                    })
                })
            })
        {
            values.entry(t).or_default().push(v);
        }
    }
    let values = values
        .into_iter()
        .filter_map(|(t, v)| {
            let d: DateTimeWrapper = OffsetDateTime::from_unix_timestamp(t).ok()?.into();
            let v_len = v.len();
            if v_len == 1 {
                Some((d, v[0]))
            } else {
                let v: i32 = v.iter().sum();
                Some((d, v / v_len as i32))
            }
        })
        .collect();
    Ok(values)
}
