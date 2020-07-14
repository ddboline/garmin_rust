use anyhow::{format_err, Error};
use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use futures::future::try_join_all;
use glob::glob;
use itertools::Itertools;
use log::debug;
use rayon::{
    iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelExtend, ParallelIterator},
    slice::ParallelSliceMut,
};
use serde::{self, Deserialize, Deserializer, Serialize};
use std::{collections::HashSet, fs::File, path::Path};
use structopt::StructOpt;

use garmin_lib::{
    common::{
        garmin_config::GarminConfig, garmin_connect_client::GarminConnectHrData,
        garmin_file::GarminFile, garmin_summary::get_list_of_files_from_db, pgpool::PgPool,
    },
    reports::garmin_templates::{PLOT_TEMPLATE, PLOT_TEMPLATE_DEMO, TIMESERIESTEMPLATE},
    utils::{iso_8601_datetime, stack_string::StackString},
};

use crate::fitbit_statistics_summary::FitbitStatisticsSummary;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq)]
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

const NMINUTES: i64 = 5;

#[derive(Deserialize, Copy, Clone)]
pub struct JsonHeartRateValue {
    pub bpm: i32,
    pub confidence: i32,
}

#[derive(Deserialize, Copy, Clone)]
pub struct JsonHeartRateEntry {
    #[serde(rename = "dateTime", deserialize_with = "deserialize_json_mdyhms")]
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
    pub fn from_json_heartrate_entry(entry: JsonHeartRateEntry) -> Self {
        Self {
            datetime: entry.datetime,
            value: entry.value.bpm,
        }
    }

    #[allow(clippy::similar_names)]
    pub async fn get_heartrate_values(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<(DateTime<Utc>, i32)>, Error> {
        let ndays = (end_date - start_date).num_days();

        let days: Vec<_> = (0..=ndays)
            .map(|i| start_date + Duration::days(i))
            .collect();
        let fitbit_files: Vec<_> = days
            .par_iter()
            .filter_map(|date| {
                let input_filename = config
                    .fitbit_cachedir
                    .join(date.to_string())
                    .with_extension("avro");
                if input_filename.exists() {
                    Some(input_filename)
                } else {
                    None
                }
            })
            .collect();
        let futures = days.iter().map(|date| async move {
            let constraint = format!("date(begin_datetime at time zone 'utc') = '{}'", date);
            let files: Vec<_> = get_list_of_files_from_db(&constraint, pool)
                .await?
                .into_par_iter()
                .filter_map(|filename| {
                    let avro_file = config.cache_dir.join(&format!("{}.avro", filename));
                    if avro_file.exists() {
                        Some(avro_file)
                    } else {
                        None
                    }
                })
                .collect();
            Ok(files)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let garmin_files: Vec<_> = results?.into_par_iter().flatten().collect();

        let results: Result<Vec<_>, Error> = fitbit_files
            .into_par_iter()
            .map(|input_path| {
                let values: Vec<_> = Self::read_avro(&input_path)?
                    .into_par_iter()
                    .map(|h| (h.datetime, h.value))
                    .collect();
                Ok(values)
            })
            .collect();
        let mut heartrate_values: Vec<_> = results?.into_par_iter().flatten().collect();

        let results: Result<Vec<_>, Error> = garmin_files
            .into_par_iter()
            .map(|avro_file| {
                let points: Vec<_> = GarminFile::read_avro(&avro_file)?
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
        Ok(heartrate_values)
    }

    pub async fn calculate_summary_statistics(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: NaiveDate,
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

    pub async fn get_all_summary_statistics(
        config: &GarminConfig,
        pool: &PgPool,
    ) -> Result<(), Error> {
        let dates: Result<Vec<_>, Error> = glob(&format!(
            "{}/*.avro",
            config.fitbit_cachedir.to_string_lossy()
        ))?
        .map(|x| {
            x.map_err(Into::into).and_then(|f| {
                let date: NaiveDate = f
                    .file_stem()
                    .ok_or_else(|| format_err!("No name"))?
                    .to_string_lossy()
                    .parse()?;
                Ok(date)
            })
        })
        .collect();
        let dates = dates?;
        let futures = dates.into_iter().map(|date| {
            let config = config.clone();
            let pool = pool.clone();
            async move {
                Self::calculate_summary_statistics(&config, &pool, date).await?;
                debug!("{}", date);
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        Ok(())
    }

    #[allow(clippy::similar_names)]
    pub async fn get_heartrate_plot(
        config: &GarminConfig,
        pool: &PgPool,
        start_date: NaiveDate,
        end_date: NaiveDate,
        is_demo: bool,
    ) -> Result<StackString, Error> {
        let mut final_values: Vec<_> =
            Self::get_heartrate_values(config, pool, start_date, end_date)
                .await?
                .into_iter()
                .group_by(|(d, _)| d.timestamp() / (NMINUTES * 60))
                .into_iter()
                .map(|(_, group)| {
                    let (begin_datetime, entries, heartrate_sum) = group.fold(
                        (None, 0, 0),
                        |(begin_datetime, entries, heartrate_sum), (datetime, heartrate)| {
                            (
                                if begin_datetime.is_none() || begin_datetime < Some(datetime) {
                                    Some(datetime)
                                } else {
                                    begin_datetime
                                },
                                entries + 1,
                                heartrate_sum + heartrate,
                            )
                        },
                    );
                    begin_datetime.map(|begin_datetime| {
                        let average_heartrate = heartrate_sum / entries;
                        let begin_datetime =
                            begin_datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
                        (begin_datetime, average_heartrate)
                    })
                })
                .collect();

        final_values.par_sort();
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
                    "{}{}{}<br>",
                    format!(
                        r#"
                        <button type="submit" id="ID"
                         onclick="heartrate_plot_date('{date}','{date}');"">Plot {date}</button>"#,
                        date = date
                    ),
                    if is_demo {
                        "".to_string()
                    } else {
                        format!(
                            r#"
                        <button type="submit" id="ID"
                         onclick="heartrate_sync('{date}');">Sync {date}</button>
                        "#,
                            date = date
                        )
                    },
                    if is_demo {
                        "".to_string()
                    } else {
                        format!(
                            r#"
                        <button type="submit" id="ID"
                         onclick="connect_hr_sync('{date}');">Sync Garmin {date}</button>
                        "#,
                            date = date
                        )
                    },
                )
            })
            .collect();
        let template = if is_demo {
            PLOT_TEMPLATE_DEMO
        } else {
            PLOT_TEMPLATE
        };
        let body = template
            .replace("INSERTOTHERIMAGESHERE", &plots)
            .replace("INSERTTEXTHERE", &buttons.join("\n"))
            .into();
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
        let input_filename = config
            .fitbit_cachedir
            .join(date.to_string())
            .with_extension("avro");
        debug!("avro {:?}", input_filename);
        if input_filename.exists() {
            Self::read_avro(&input_filename)
        } else {
            Ok(Vec::new())
        }
    }

    pub fn read_avro(input_filename: &Path) -> Result<Vec<Self>, Error> {
        let input_file = File::open(input_filename)?;

        Reader::new(input_file)
            .map_err(|e| format_err!("{}", e))?
            .next()
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
            let mut merged_values: Vec<_> = Self::read_avro_by_date(config, date)?
                .into_par_iter()
                .chain(new_values)
                .collect();
            merged_values.par_sort_by_key(|entry| entry.datetime.timestamp());
            merged_values.dedup();
            let input_filename = config
                .fitbit_cachedir
                .join(date.to_string())
                .with_extension("avro");
            Self::dump_to_avro(&merged_values, &input_filename.to_string_lossy())?;
        }
        Ok(())
    }

    pub fn from_garmin_connect_hr(hr_data: &GarminConnectHrData) -> Vec<Self> {
        if let Some(hr_vals) = hr_data.heartrate_values.as_ref() {
            hr_vals
                .iter()
                .filter_map(|(timestamp_ms, hr_val_opt)| {
                    hr_val_opt.map(|value| {
                        let timestamp: i64 = timestamp_ms / 1000;
                        let datetime = DateTime::<Utc>::from_utc(
                            NaiveDateTime::from_timestamp(
                                timestamp,
                                ((timestamp_ms - timestamp * 1000) * 1_000_000) as u32,
                            ),
                            Utc,
                        );
                        Self { datetime, value }
                    })
                })
                .collect()
        } else {
            Vec::new()
        }
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
    pub directory: StackString,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct FitbitBodyWeightFat {
    pub datetime: DateTime<Utc>,
    pub weight: f64,
    pub fat: f64,
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::NaiveDate;
    use log::debug;
    use std::{collections::HashSet, path::Path};

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::fitbit_heartrate::{process_fitbit_json_file, FitbitHeartRate};

    #[test]
    #[ignore]
    fn test_process_fitbit_json_file() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let path = Path::new("tests/data/test_heartrate_data.json");
        let result = process_fitbit_json_file(&path)?;
        debug!("{}", result.len());

        let dates: HashSet<_> = result
            .iter()
            .map(|entry| entry.datetime.date().naive_local())
            .collect();
        debug!("{:?}", dates);
        let dates = vec![NaiveDate::from_ymd(2019, 11, 1)];
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
    async fn test_get_heartrate_plot() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let start_date = NaiveDate::from_ymd(2019, 8, 1);
        let end_date = NaiveDate::from_ymd(2019, 8, 2);
        let results =
            FitbitHeartRate::get_heartrate_plot(&config, &pool, start_date, end_date, false)
                .await?;
        debug!("{}", results);
        assert!(results.len() > 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate_summary_statistics() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let start_date = NaiveDate::from_ymd(2019, 8, 1);
        let result =
            FitbitHeartRate::calculate_summary_statistics(&config, &pool, start_date).await?;
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.min_heartrate as i32, 39);
        assert_eq!(result.max_heartrate as i32, 181);
        assert_eq!(result.median_heartrate as i32, 61);
        assert_eq!(result.number_of_entries as i32, 11393);
        Ok(())
    }
}
