use anyhow::{format_err, Error};
use chrono::{Duration, NaiveDate, Utc};
use futures::future::try_join_all;
use itertools::Itertools;
use refinery::embed_migrations;
use stack_string::StackString;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::{
    fs::{read_to_string, File},
    io::{stdin, stdout, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::spawn_blocking,
};

use fitbit_lib::{
    fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate,
    fitbit_statistics_summary::FitbitStatisticsSummary, scale_measurement::ScaleMeasurement,
};
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::{
    fitbit_activity::FitbitActivity, garmin_config::GarminConfig,
    garmin_connect_activity::GarminConnectActivity, garmin_summary::get_maximum_begin_datetime,
    pgpool::PgPool, strava_activity::StravaActivity,
};
use race_result_analysis::{race_results::RaceResults, race_type::RaceType};
use strava_lib::strava_client::StravaClient;

use crate::garmin_cli::{GarminCli, GarminCliOptions};

embed_migrations!("../migrations");

#[derive(StructOpt, PartialEq)]
pub enum GarminCliOpts {
    #[structopt(alias = "boot")]
    Bootstrap,
    Proc {
        #[structopt(short, long)]
        filename: Vec<PathBuf>,
    },
    #[structopt(alias = "rpt")]
    Report {
        #[structopt(short, long)]
        patterns: Vec<StackString>,
    },
    #[structopt(alias = "cnt")]
    Connect {
        #[structopt(short, long)]
        start_date: Option<NaiveDate>,
        #[structopt(short, long)]
        end_date: Option<NaiveDate>,
    },
    Sync {
        #[structopt(short, long)]
        md5sum: bool,
    },
    #[structopt(alias = "fit")]
    Fitbit {
        #[structopt(short, long)]
        all: bool,
        #[structopt(short, long)]
        start_date: Option<NaiveDate>,
        #[structopt(short, long)]
        end_date: Option<NaiveDate>,
    },
    Strava,
    Import {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    SyncAll,
    /// Run refinery migrations
    RunMigrations,
}

impl GarminCliOpts {
    pub async fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = Self::from_args();

        if opts == Self::SyncAll {
            Self::Connect {
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
            Self::Sync { md5sum: false }.process_opts(&config).await
        } else {
            opts.process_opts(&config).await
        }
    }

    async fn process_opts(self, config: &GarminConfig) -> Result<(), Error> {
        let pool = PgPool::new(&config.pgurl);

        let opts = match self {
            Self::Bootstrap => GarminCliOptions::Bootstrap,
            Self::Proc { filename } => GarminCliOptions::ImportFileNames(filename),
            Self::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(&config, &["year".to_string()])
                } else {
                    GarminCli::process_pattern(&config, &patterns)
                };
                let cli = GarminCli::with_config()?;
                cli.run_cli(&req.options, &req.constraints).await?;
                return cli.stdout.close().await;
            }
            Self::Connect {
                start_date,
                end_date,
            } => {
                if start_date > end_date {
                    return Err(format_err!("Invalid date range"));
                }
                GarminCliOptions::Connect {
                    start_date,
                    end_date,
                }
            }
            Self::Sync { md5sum } => GarminCliOptions::Sync(md5sum),
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
                let start_date = start_date
                    .unwrap_or_else(|| (Utc::now() - Duration::days(3)).naive_utc().date());
                let end_date = end_date.unwrap_or_else(|| Utc::now().naive_utc().date());
                let mut date = start_date;
                while date <= end_date {
                    client.import_fitbit_heartrate(date).await?;
                    FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, date)
                        .await?;
                    date += Duration::days(1);
                }
                cli.stdout.send(format!("{:?}", updates));

                let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
                let filenames = client
                    .sync_tcx(start_date)
                    .await?
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .join("\n");
                cli.stdout.send(filenames);

                cli.stdout.close().await?;

                if all {
                    FitbitHeartRate::get_all_summary_statistics(&client.config, &pool).await?;
                }
                return cli.stdout.close().await;
            }
            Self::Strava => {
                let cli = GarminCli::with_config()?;
                let filenames = Self::sync_with_strava(&cli)
                    .await?
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .join("\n");
                cli.stdout.send(filenames);
                return cli.stdout.close().await;
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
                                format!("scale_measurements {}\n", measurements.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "strava_activities" => {
                        let activities: Vec<StravaActivity> = serde_json::from_str(&data)?;
                        StravaActivity::upsert_activities(&activities, &pool).await?;
                        StravaActivity::fix_summary_id_in_db(&pool).await?;
                        stdout()
                            .write_all(
                                format!("strava_activities {}\n", activities.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "fitbit_activities" => {
                        let activities: Vec<FitbitActivity> = serde_json::from_str(&data)?;
                        FitbitActivity::upsert_activities(&activities, &pool).await?;
                        FitbitActivity::fix_summary_id_in_db(&pool).await?;
                        stdout()
                            .write_all(
                                format!("fitbit_activities {}\n", activities.len()).as_bytes(),
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
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(
                                format!("heartrate_statistics_summary {}\n", results?.len())
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
                                format!("garmin_connect_activities {}\n", activities.len())
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
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(format!("race_results {}\n", results?.len()).as_bytes())
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
                match table.as_str() {
                    "scale_measurements" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        let measurements =
                            ScaleMeasurement::read_from_db(&pool, Some(start_date), None).await?;
                        file.write_all(&serde_json::to_vec(&measurements)?).await?;
                    }
                    "strava_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        let activities =
                            StravaActivity::read_from_db(&pool, Some(start_date), None).await?;
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "fitbit_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        let activities =
                            FitbitActivity::read_from_db(&pool, Some(start_date), None).await?;
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "heartrate_statistics_summary" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        let entries =
                            FitbitStatisticsSummary::read_from_db(Some(start_date), None, &pool)
                                .await?;
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    "garmin_connect_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        let activities =
                            GarminConnectActivity::read_from_db(&pool, Some(start_date), None)
                                .await?;
                        file.write_all(&serde_json::to_vec(&activities)?).await?;
                    }
                    "race_results" => {
                        file.write_all(&serde_json::to_vec(
                            &RaceResults::get_results_by_type(RaceType::Personal, &pool).await?,
                        )?)
                        .await?;
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
        };

        let cli = GarminCli {
            opts: Some(opts),
            pool,
            config: config.clone(),
            ..GarminCli::with_config()?
        };

        Self::garmin_proc(&cli).await?;
        cli.stdout.close().await
    }

    pub async fn garmin_proc(cli: &GarminCli) -> Result<(), Error> {
        if let Some(GarminCliOptions::Connect {
            start_date,
            end_date,
        }) = cli.get_opts()
        {
            Self::sync_with_garmin_connect(&cli, *start_date, *end_date).await?;
        }

        if let Some(GarminCliOptions::ImportFileNames(filenames)) = cli.get_opts() {
            let filenames = filenames.clone();

            cli.process_filenames(&filenames).await?;
        }

        let results = match cli.get_opts() {
            Some(GarminCliOptions::Bootstrap) => cli.run_bootstrap().await,
            Some(GarminCliOptions::Sync(check_md5)) => cli.sync_everything(*check_md5).await,
            _ => cli.proc_everything().await,
        }?;
        cli.stdout.send(results.join("\n"));
        Ok(())
    }

    pub async fn sync_with_garmin_connect(
        cli: &GarminCli,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<PathBuf>, Error> {
        if let Some(max_datetime) = get_maximum_begin_datetime(&cli.pool).await? {
            let mut session = GarminConnectClient::new(cli.config.clone());
            session.init().await?;
            let activities = session.get_activities(Some(max_datetime)).await?;
            let start_date =
                start_date.unwrap_or_else(|| (Utc::now() - Duration::days(3)).naive_utc().date());
            let end_date = end_date.unwrap_or_else(|| Utc::now().naive_utc().date());
            let mut date = start_date;
            while date <= end_date {
                let hr_values = session.get_heartrate(date).await?;
                let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                let config = cli.config.clone();
                spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &hr_values))
                    .await??;
                FitbitHeartRate::calculate_summary_statistics(&cli.config, &cli.pool, date).await?;
                date += Duration::days(1);
            }
            let filenames = session
                .get_and_merge_activity_files(activities, &cli.pool)
                .await?;
            if !filenames.is_empty() {
                cli.process_filenames(&filenames).await?;
                cli.proc_everything().await?;
                GarminConnectActivity::fix_summary_id_in_db(&cli.pool).await?;
            }
            session.close().await?;
            Ok(filenames)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn sync_with_strava(cli: &GarminCli) -> Result<Vec<PathBuf>, Error> {
        let config = cli.config.clone();
        let start_datetime = Some(Utc::now() - Duration::days(15));
        let end_datetime = Some(Utc::now());

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
