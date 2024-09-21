use anyhow::{format_err, Error};
use clap::Parser;
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use log::info;
use refinery::embed_migrations;
use stack_string::{format_sstr, StackString};
use std::{collections::BTreeSet, ffi::OsStr, path::PathBuf};
use tempdir::TempDir;
use time::{macros::format_description, Date, Duration, OffsetDateTime};
use time_tz::OffsetDateTimeExt;
use tokio::{
    fs::{read_to_string, remove_file, write, File},
    io::{stdin, stdout, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::spawn_blocking,
};

use derive_more::{From, Into};
use fitbit_lib::{
    fitbit_archive::{
        archive_fitbit_heartrates, get_heartrate_values, get_number_of_heartrate_values,
    },
    fitbit_client::FitbitClient,
    fitbit_heartrate::{import_garmin_heartrate_file, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
    GarminConnectHrData,
};
use garmin_lib::{date_time_wrapper::DateTimeWrapper, garmin_config::GarminConfig};
use garmin_models::{
    fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
    garmin_connect_har_file::GarminConnectHarFile, strava_activity::StravaActivity,
};
use garmin_utils::{garmin_util::extract_zip_from_garmin_connect_multiple, pgpool::PgPool};
use race_result_analysis::{race_results::RaceResults, race_type::RaceType};
use std::str::FromStr;
use strava_lib::strava_client::StravaClient;

use crate::garmin_cli::{GarminCli, GarminCliOptions};

embed_migrations!("../migrations");

#[derive(Into, From, Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct DateType(Date);

impl FromStr for DateType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Date::parse(s, format_description!("[year]-[month]-[day]"))
            .map(Self)
            .map_err(Into::into)
    }
}

#[derive(Parser, PartialEq, Eq)]
pub enum GarminCliOpts {
    #[clap(alias = "boot")]
    Bootstrap,
    Proc {
        #[clap(short, long, use_value_delimiter = true, value_delimiter = ',')]
        filename: Vec<PathBuf>,
    },
    #[clap(alias = "rpt")]
    Report {
        #[clap(short, long, use_value_delimiter = true, value_delimiter = ',')]
        patterns: Vec<StackString>,
    },
    #[clap(alias = "cnt")]
    /// Go to the network tab in chrome developer tools,
    /// Navigate to `<https://connect.garmin.com/modern/activities>`,
    /// find the entry for `<https://connect.garmin.com/activitylist-service/activities/search/activities>`,
    /// go to the response subtab and copy the output to
    /// ~/Downloads/garmin_connect/activities.json, Next navigate to `<https://connect.garmin.com/modern/daily-summary/{date}>` where date is a date e.g. 2022-12-20,
    /// find the entry `<https://connect.garmin.com/wellness-service/wellness/dailyHeartRate/ddboline?date=2022-12-18>`,
    /// go to the response subtab and copy the output to
    /// `~/Downloads/garmin_connect/heartrates.json``
    Connect {
        #[clap(short, long)]
        data_directory: Option<PathBuf>,
        #[clap(short, long)]
        start_date: Option<DateType>,
        #[clap(short, long)]
        end_date: Option<DateType>,
    },
    Sync,
    #[clap(alias = "fit")]
    Fitbit {
        #[clap(short, long)]
        all: bool,
        #[clap(short, long)]
        start_date: Option<DateType>,
        #[clap(short, long)]
        end_date: Option<DateType>,
    },
    Strava,
    Import {
        #[clap(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: StackString,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[clap(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: StackString,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
    },
    SyncAll,
    /// Run refinery migrations
    RunMigrations,
    #[clap(alias = "archive")]
    FitbitArchive {
        #[clap(short, long)]
        all: bool,
    },
    #[clap(alias = "fit-archive-read")]
    FitbitArchiveRead {
        #[clap(short, long)]
        start_date: Option<DateType>,
        #[clap(short, long)]
        end_date: Option<DateType>,
        #[clap(short = 't', long)]
        step_size: Option<usize>,
    },
}

impl GarminCliOpts {
    /// # Errors
    /// Return error if config fails, or `process_opts` fails
    pub async fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = Self::parse();

        if opts == Self::SyncAll {
            Self::Connect {
                data_directory: None,
                start_date: None,
                end_date: None,
            }
            .process_opts(&config)
            .await?;
            Self::Fitbit {
                all: false,
                start_date: None,
                end_date: None,
            }
            .process_opts(&config)
            .await?;
            Self::Strava.process_opts(&config).await?;
            Self::Sync.process_opts(&config).await
        } else {
            opts.process_opts(&config).await
        }
    }

    async fn process_opts(self, config: &GarminConfig) -> Result<(), Error> {
        let pool = PgPool::new(&config.pgurl)?;

        let opts = match self {
            Self::Bootstrap => GarminCliOptions::Bootstrap,
            Self::Proc { filename } => GarminCliOptions::ImportFileNames(filename),
            Self::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(config, ["year"])
                } else {
                    GarminCli::process_pattern(config, &patterns)
                };
                let cli = GarminCli::with_config()?;
                cli.run_cli(&req.options, &req.constraints).await?;
                return cli.stdout.close().await.map_err(Into::into);
            }
            Self::Connect {
                data_directory,
                start_date,
                end_date,
            } => GarminCliOptions::Connect {
                data_directory,
                start_date: start_date.map(Into::into),
                end_date: end_date.map(Into::into),
            },
            Self::Sync => GarminCliOptions::Sync,
            Self::SyncAll => {
                return Ok(());
            }
            Self::Fitbit {
                all,
                start_date,
                end_date,
            } => {
                if start_date > end_date {
                    return Err(format_err!("Invalid date range"));
                }
                let cli = GarminCli::with_config()?;
                let client = FitbitClient::with_auth(config.clone()).await?;
                let updates = client.sync_everything(&pool).await?;
                let start_date = start_date.map_or_else(
                    || (OffsetDateTime::now_utc() - Duration::days(3)).date(),
                    Into::into,
                );
                let end_date =
                    end_date.map_or_else(|| OffsetDateTime::now_utc().date(), Into::into);
                let mut date = start_date;
                while date <= end_date {
                    client.import_fitbit_heartrate(date).await?;
                    FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, date)
                        .await?;
                    date += Duration::days(1);
                }
                cli.stdout.send(format_sstr!("{updates:?}"));

                let start_date = (OffsetDateTime::now_utc() - Duration::days(10)).date();
                let filenames = client.sync_tcx(start_date).await?;
                if !filenames.is_empty() {
                    let mut buf = cli.proc_everything().await?;
                    buf.extend_from_slice(&cli.sync_everything().await?);
                }
                let filenames = filenames
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .join("\n");
                cli.stdout.send(filenames);

                if all {
                    FitbitHeartRate::get_all_summary_statistics(&client.config, &pool).await?;
                }
                return cli.stdout.close().await.map_err(Into::into);
            }
            Self::Strava => {
                let cli = GarminCli::with_config()?;
                let filenames = Self::sync_with_strava(&cli)
                    .await?
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .join("\n");
                cli.stdout.send(filenames);
                return cli.stdout.close().await.map_err(Into::into);
            }
            Self::Import { table, filepath } => {
                let data = if let Some(filepath) = filepath {
                    read_to_string(&filepath).await?
                } else {
                    let mut stdin = stdin();
                    let mut buf = String::new();
                    stdin.read_to_string(&mut buf).await?;
                    buf
                };
                match table.as_str() {
                    "scale_measurements" => {
                        let mut measurements: Vec<ScaleMeasurement> = serde_json::from_str(&data)?;
                        ScaleMeasurement::merge_updates(&mut measurements, &pool).await?;
                        stdout()
                            .write_all(
                                format_sstr!("scale_measurements {}\n", measurements.len())
                                    .as_bytes(),
                            )
                            .await?;
                    }
                    "strava_activities" => {
                        let activities: Vec<StravaActivity> = serde_json::from_str(&data)?;
                        StravaActivity::upsert_activities(&activities, &pool).await?;
                        StravaActivity::fix_summary_id_in_db(&pool).await?;
                        stdout()
                            .write_all(
                                format_sstr!("strava_activities {}\n", activities.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "fitbit_activities" => {
                        let activities: Vec<FitbitActivity> = serde_json::from_str(&data)?;
                        FitbitActivity::upsert_activities(&activities, &pool).await?;
                        FitbitActivity::fix_summary_id_in_db(&pool).await?;
                        stdout()
                            .write_all(
                                format_sstr!("fitbit_activities {}\n", activities.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "heartrate_statistics_summary" => {
                        let entries: Vec<FitbitStatisticsSummary> = serde_json::from_str(&data)?;
                        let futures = entries.into_iter().map(|entry| {
                            let pool = pool.clone();
                            async move {
                                FitbitStatisticsSummary::upsert_entry(&entry, &pool).await?;
                                Ok(())
                            }
                        });
                        let results: Result<Vec<()>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(
                                format_sstr!("heartrate_statistics_summary {}\n", results?.len())
                                    .as_bytes(),
                            )
                            .await?;
                    }
                    "garmin_connect_activities" => {
                        let activities: Vec<GarminConnectActivity> = serde_json::from_str(&data)?;
                        GarminConnectActivity::upsert_activities(&activities, &pool).await?;
                        GarminConnectActivity::fix_summary_id_in_db(&pool).await?;
                        stdout()
                            .write_all(
                                format_sstr!("garmin_connect_activities {}\n", activities.len())
                                    .as_bytes(),
                            )
                            .await?;
                    }
                    "race_results" => {
                        let results: Vec<RaceResults> = serde_json::from_str(&data)?;
                        let futures = results.into_iter().map(|result| {
                            let pool = pool.clone();
                            async move {
                                result.update_db(&pool).await?;
                                Ok(())
                            }
                        });
                        let results: Result<Vec<()>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(format_sstr!("race_results {}\n", results?.len()).as_bytes())
                            .await?;
                    }
                    _ => {}
                }
                return Ok(());
            }
            Self::Export { table, filepath } => {
                let mut file: Box<dyn AsyncWrite + Unpin> = if let Some(filepath) = filepath {
                    Box::new(File::create(&filepath).await?)
                } else {
                    Box::new(stdout())
                };
                let local = DateTimeWrapper::local_tz();
                match table.as_str() {
                    "scale_measurements" => {
                        let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                            .to_timezone(local)
                            .date();
                        let measurements =
                            ScaleMeasurement::read_from_db(&pool, Some(start_date), None).await?;
                        file.write_all(&serde_json::to_vec(&measurements)?).await?;
                    }
                    "strava_activities" => {
                        let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                            .to_timezone(local)
                            .date();
                        let mut activities: Vec<_> =
                            StravaActivity::read_from_db(&pool, Some(start_date), None)
                                .await?
                                .try_collect()
                                .await?;
                        activities.shrink_to_fit();
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "fitbit_activities" => {
                        let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                            .to_timezone(local)
                            .date();
                        let activities =
                            FitbitActivity::read_from_db(&pool, Some(start_date), None).await?;
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "heartrate_statistics_summary" => {
                        let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                            .to_timezone(local)
                            .date();
                        let mut entries: Vec<_> =
                            FitbitStatisticsSummary::read_from_db(Some(start_date), None, &pool)
                                .await?
                                .try_collect()
                                .await?;
                        entries.shrink_to_fit();
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    "garmin_connect_activities" => {
                        let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                            .to_timezone(local)
                            .date();
                        let mut activities: Vec<_> =
                            GarminConnectActivity::read_from_db(&pool, Some(start_date), None)
                                .await?
                                .try_collect()
                                .await?;
                        activities.shrink_to_fit();
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "race_results" => {
                        let mut results: Vec<_> =
                            RaceResults::get_results_by_type(RaceType::Personal, &pool)
                                .await?
                                .try_collect()
                                .await?;
                        results.shrink_to_fit();
                        file.write_all(&serde_json::to_vec(&results)?).await?;
                    }
                    _ => {}
                }

                return Ok(());
            }
            Self::RunMigrations => {
                let mut client = pool.get().await?;
                migrations::runner().run_async(&mut **client).await?;
                return Ok(());
            }
            Self::FitbitArchive { all } => {
                let result = archive_fitbit_heartrates(config, &pool, all).await?;
                stdout().write_all(result.join("\n").as_bytes()).await?;
                stdout().write_all(b"\n").await?;
                return Ok(());
            }
            Self::FitbitArchiveRead {
                start_date,
                end_date,
                step_size,
            } => {
                let start_date = start_date.map_or_else(
                    || (OffsetDateTime::now_utc() - Duration::days(1)).date(),
                    Into::into,
                );
                let end_date = end_date.map_or_else(
                    || (OffsetDateTime::now_utc() + Duration::days(1)).date(),
                    Into::into,
                );
                let count = {
                    let config = config.clone();
                    spawn_blocking(move || {
                        get_number_of_heartrate_values(&config, start_date, end_date)
                    })
                    .await??
                };
                let config = config.clone();
                let values = spawn_blocking(move || {
                    get_heartrate_values(&config, start_date, end_date, step_size)
                })
                .await??;
                let mut values: Vec<_> = values
                    .into_iter()
                    .map(|(d, v)| format_sstr!("{d} {v}"))
                    .collect();
                values.shrink_to_fit();
                stdout()
                    .write_all(format_sstr!("count {count} {}", values.len()).as_bytes())
                    .await?;
                stdout().write_all(b"\n").await?;
                return Ok(());
            }
        };

        let cli = GarminCli {
            opts: Some(opts),
            pool,
            config: config.clone(),
            ..GarminCli::with_config()?
        };

        Self::garmin_proc(&cli).await?;
        cli.stdout.close().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn garmin_proc(cli: &GarminCli) -> Result<(), Error> {
        let results = match cli.get_opts() {
            Some(GarminCliOptions::ImportFileNames(filenames)) => {
                let filenames = filenames.clone();
                cli.process_filenames(&filenames).await?;
                cli.proc_everything().await
            }
            Some(GarminCliOptions::Bootstrap) => cli.run_bootstrap().await,
            Some(GarminCliOptions::Sync) => cli.sync_everything().await,
            Some(GarminCliOptions::Connect {
                data_directory,
                start_date,
                end_date,
            }) => {
                let mut buf = cli.proc_everything().await?;
                let (filenames, input_files, dates) =
                    Self::sync_with_garmin_connect(cli, data_directory, *start_date, *end_date)
                        .await?;
                if !filenames.is_empty() || !input_files.is_empty() || !dates.is_empty() {
                    buf.extend_from_slice(&cli.sync_everything().await?);
                    if let Ok(client) = FitbitClient::with_auth(cli.config.clone()).await {
                        let result = client.sync_everything(&cli.pool).await?;
                        buf.push(format_sstr!(
                            "Syncing Fitbit Heartrate {hr} Activities {ac} Duplicates {dp}",
                            hr = result.measurements.len(),
                            ac = result.activities.len(),
                            dp = result.duplicates.len(),
                        ));
                    }
                }
                Ok(buf)
            }
            _ => cli.proc_everything().await,
        }?;
        cli.stdout.send(results.join("\n"));
        Ok(())
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn sync_with_garmin_connect(
        cli: &GarminCli,
        data_directory: &Option<PathBuf>,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>, Vec<Date>), Error> {
        let mut input_files = Vec::new();
        let mut filenames = Vec::new();
        let mut activities = Vec::new();
        let mut dates = BTreeSet::new();
        let har_file = cli.config.download_directory.join("connect.garmin.com.har");
        if har_file.exists() {
            let buf = read_to_string(&har_file).await?;
            let har: GarminConnectHarFile = serde_json::from_str(buf.trim())?;
            activities = har.get_activities()?;
            for buf in har.get_heartrates() {
                let hr_values: GarminConnectHrData = serde_json::from_str(buf)?;
                let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                let config = cli.config.clone();
                dates.extend(
                    spawn_blocking(move || {
                        FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                    })
                    .await??,
                );
            }
            input_files.push(har_file);
        }
        let data_directory = data_directory
            .as_ref()
            .unwrap_or(&cli.config.garmin_connect_import_directory);
        let activites_json = data_directory.join("activities.json");
        if activities.is_empty() && activites_json.exists() {
            let buf = read_to_string(&activites_json).await?;
            if !buf.is_empty() {
                activities = serde_json::from_str(buf.trim())?;
                activities =
                    GarminConnectActivity::merge_new_activities(activities, &cli.pool).await?;
                input_files.push(activites_json);
            }
        }
        for activity in activities {
            let filename = cli
                .config
                .download_directory
                .join(format_sstr!("{}.zip", activity.activity_id));
            if filename.exists() {
                filenames.push(filename);
            }
        }
        let heartrate_json = data_directory.join("heartrates.json");
        if heartrate_json.exists() {
            let buf = read_to_string(&heartrate_json).await?;
            for line in buf.split('\n') {
                if line.is_empty() {
                    continue;
                }
                if let Ok(hr_values) = serde_json::from_str::<GarminConnectHrData>(line) {
                    let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                    let config = cli.config.clone();
                    dates.extend(
                        spawn_blocking(move || {
                            FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                        })
                        .await??,
                    );
                }
            }
            if !buf.is_empty() {
                input_files.push(heartrate_json);
            }
        }
        let start_date =
            start_date.unwrap_or_else(|| (OffsetDateTime::now_utc() - Duration::days(3)).date());
        let end_date = end_date.unwrap_or_else(|| OffsetDateTime::now_utc().date());
        let mut date = start_date;
        while date <= end_date {
            info!("get heartrate {date}");
            let heartrate_file = data_directory.join(format_sstr!("{date}.json"));
            if heartrate_file.exists() {
                let hr_values: GarminConnectHrData =
                    serde_json::from_reader(File::open(&heartrate_file).await?.into_std().await)?;
                info!("got heartrate {date}");
                let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                let config = cli.config.clone();
                let mut new_dates = spawn_blocking(move || {
                    FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                })
                .await??;
                dates.append(&mut new_dates);
                remove_file(&heartrate_file).await?;
            }
            let connect_wellness_file = data_directory.join(format_sstr!("{date}"));
            let connect_wellness_file = if connect_wellness_file.exists() {
                connect_wellness_file
            } else {
                cli.config.download_directory.join(format_sstr!("{date}"))
            };
            if connect_wellness_file.exists() {
                let tempdir = TempDir::new("garmin_zip")?;
                let ziptmpdir = tempdir.path().to_path_buf();
                let wellness_files = spawn_blocking(move || {
                    extract_zip_from_garmin_connect_multiple(&connect_wellness_file, &ziptmpdir)
                })
                .await??;
                for wellness_file in wellness_files {
                    let config = cli.config.clone();
                    if let Ok(mut new_dates) = spawn_blocking(move || {
                        import_garmin_heartrate_file(&config, &wellness_file)
                    })
                    .await?
                    {
                        dates.append(&mut new_dates);
                    }
                }
            }
            date += Duration::days(1);
        }
        for date in &dates {
            if let Some(stat) =
                FitbitHeartRate::calculate_summary_statistics(&cli.config, &cli.pool, *date).await?
            {
                info!("update stats {}", stat.date);
            }
        }
        if !filenames.is_empty() {
            let datetimes = cli.process_filenames(&filenames).await?;
            info!("number of files {}", datetimes.len());
        }
        if !filenames.is_empty() || !input_files.is_empty() {
            for line in cli.proc_everything().await? {
                info!("{line}");
            }
            GarminConnectActivity::fix_summary_id_in_db(&cli.pool).await?;
        }
        for line in archive_fitbit_heartrates(&cli.config, &cli.pool, false).await? {
            info!("{line}");
        }
        for f in &input_files {
            if f.extension() == Some(OsStr::new("har")) {
                remove_file(f).await?;
            } else {
                write(f, &[]).await?;
            }
        }
        let mut dates: Vec<_> = dates.into_iter().collect();
        dates.shrink_to_fit();
        Ok((filenames, input_files, dates))
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn sync_with_strava(cli: &GarminCli) -> Result<Vec<PathBuf>, Error> {
        let config = cli.config.clone();
        let start_datetime = Some(OffsetDateTime::now_utc() - Duration::days(15));
        let end_datetime = Some(OffsetDateTime::now_utc());

        let client = StravaClient::with_auth(config).await?;
        let filenames = client
            .sync_with_client(start_datetime, end_datetime, &cli.pool)
            .await?;

        if !filenames.is_empty() {
            cli.process_filenames(&filenames).await?;
            StravaActivity::fix_summary_id_in_db(&cli.pool).await?;
            cli.proc_everything().await?;
        }

        Ok(filenames)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use std::{ffi::OsStr, path::Path};
    use stdout_channel::StdoutChannel;

    use crate::garmin_cli::{GarminCli, GarminCliOptions};
    use fitbit_lib::GarminConnectHrData;
    use garmin_lib::garmin_config::GarminConfig;
    use garmin_models::{
        garmin_connect_har_file::GarminConnectHarFile, garmin_correction_lap::GarminCorrectionMap,
    };
    use garmin_parser::garmin_parse::GarminParse;
    use garmin_utils::pgpool::PgPool;

    #[tokio::test]
    #[ignore]
    async fn test_garmin_file_test_filenames() -> Result<(), Error> {
        let test_config = "tests/data/test.env";
        let config = GarminConfig::get_config(Some(test_config))?;
        let pool = PgPool::new(&config.pgurl)?;
        let corr = GarminCorrectionMap::new();

        let gcli = GarminCli {
            config,
            opts: Some(GarminCliOptions::FileNames(vec![
                "../tests/data/test.fit".into(),
                "../tests/data/test.gmn".into(),
                "../tests/data/test.tcx".into(),
                "../tests/data/test.txt".into(),
                "../tests/data/test.tcx.gz".into(),
            ])),
            pool,
            corr,
            parser: GarminParse::new(),
            stdout: StdoutChannel::new(),
        };

        assert!(gcli.opts.is_some());
        Ok(())
    }

    #[test]
    fn test_garmin_connect_har_file() -> Result<(), Error> {
        let buf = include_str!("../../tests/data/connect.garmin.com.har");
        let har: GarminConnectHarFile = serde_json::from_str(buf)?;
        let activities = har.get_activities()?;
        assert_eq!(activities.len(), 20);
        let mut total = 0;
        for buf in har.get_heartrates() {
            let hr_values: GarminConnectHrData = serde_json::from_str(buf)?;
            assert!(hr_values.heartrate_values.is_some());
            let hr_values = hr_values.heartrate_values.unwrap();
            assert!(hr_values.len() > 0);
            total += hr_values.len();
        }
        assert_eq!(total, 1187);

        let p = Path::new("../../tests/data/connect.garmin.com.har");
        assert_eq!(p.extension(), Some(OsStr::new("har")));
        Ok(())
    }
}
