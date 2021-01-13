use anyhow::Error;
use chrono::{Duration, Utc};
use futures::future::try_join_all;
use itertools::Itertools;
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
    scale_measurement::ScaleMeasurement,
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
    Connect,
    Sync {
        #[structopt(short, long)]
        md5sum: bool,
    },
    #[structopt(alias = "fit")]
    Fitbit {
        #[structopt(short, long)]
        all: bool,
    },
    Strava,
    Import {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    SyncAll,
}

impl GarminCliOpts {
    pub async fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = Self::from_args();

        if opts == Self::SyncAll {
            Self::Connect.process_opts(&config).await?;
            Self::Fitbit { all: false }.process_opts(&config).await?;
            Self::Strava.process_opts(&config).await?;
            Self::Sync { md5sum: false }.process_opts(&config).await
        } else {
            opts.process_opts(&config).await
        }
    }

    async fn process_opts(self, config: &GarminConfig) -> Result<(), Error> {
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
            Self::Connect => GarminCliOptions::Connect,
            Self::Sync { md5sum } => GarminCliOptions::Sync(md5sum),
            Self::SyncAll => {
                return Ok(());
            }
            Self::Fitbit { all } => {
                let today = Utc::now().naive_local().date();

                let cli = GarminCli::with_config()?;
                let config = GarminConfig::get_config(None)?;
                let pool = PgPool::new(&config.pgurl);
                let client = FitbitClient::with_auth(config.clone()).await?;
                let updates = client.sync_everything(&pool).await?;
                for idx in 0..3 {
                    let date = (Utc::now() - Duration::days(idx)).naive_utc().date();
                    client.import_fitbit_heartrate(date).await?;
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
                FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, today).await?;

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
                let config = GarminConfig::get_config(None)?;
                let pool = PgPool::new(&config.pgurl);
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
                        let measurements: Vec<ScaleMeasurement> = serde_json::from_str(&data)?;
                        ScaleMeasurement::merge_updates(&measurements, &pool).await?;
                        stdout()
                            .write_all(
                                format!("scale_measurements {}\n", measurements.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "strava_activities" => {
                        let activities: Vec<StravaActivity> = serde_json::from_str(&data)?;
                        StravaActivity::upsert_activities(&activities, &pool).await?;
                        stdout()
                            .write_all(
                                format!("strava_activities {}\n", activities.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "fitbit_activities" => {
                        let activities: Vec<FitbitActivity> = serde_json::from_str(&data)?;
                        FitbitActivity::upsert_activities(&activities, &pool).await?;
                        stdout()
                            .write_all(
                                format!("fitbit_activities {}\n", activities.len()).as_bytes(),
                            )
                            .await?;
                    }
                    "garmin_connect_activities" => {
                        let activities: Vec<GarminConnectActivity> = serde_json::from_str(&data)?;
                        GarminConnectActivity::upsert_activities(&activities, &pool).await?;
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
                            async move { result.update_db(&pool).await.map_err(Into::into) }
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
                let config = GarminConfig::get_config(None)?;
                let pool = PgPool::new(&config.pgurl);
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
        };

        let cli = GarminCli {
            opts: Some(opts),
            ..GarminCli::with_config()?
        };

        Self::garmin_proc(&cli).await?;
        cli.stdout.close().await
    }

    pub async fn garmin_proc(cli: &GarminCli) -> Result<(), Error> {
        if let Some(GarminCliOptions::Connect) = cli.get_opts() {
            Self::sync_with_garmin_connect(&cli).await?;
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

    pub async fn sync_with_garmin_connect(cli: &GarminCli) -> Result<Vec<PathBuf>, Error> {
        if let Some(max_datetime) = get_maximum_begin_datetime(&cli.pool).await? {
            let mut session = GarminConnectClient::new(cli.config.clone());
            session.init().await?;

            let activities = session.get_activities(max_datetime).await?;

            let activities =
                GarminConnectActivity::merge_new_activities(activities, &cli.pool).await?;

            let hr_values = session
                .get_heartrate((Utc::now()).naive_local().date())
                .await?;
            let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
            let config = cli.config.clone();
            spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &hr_values))
                .await??;

            if let Ok(filenames) = session.get_activity_files(&activities).await {
                if !filenames.is_empty() {
                    cli.process_filenames(&filenames).await?;
                    cli.proc_everything().await?;
                }
                session.close().await?;
                return Ok(filenames);
            }
            session.close().await?;
        }
        Ok(Vec::new())
    }

    pub async fn sync_with_strava(cli: &GarminCli) -> Result<Vec<PathBuf>, Error> {
        let config = cli.config.clone();
        let pool = PgPool::new(&config.pgurl);
        let start_datetime = Some(Utc::now() - Duration::days(15));
        let end_datetime = Some(Utc::now());

        let client = StravaClient::with_auth(config).await?;
        let filenames = client
            .sync_with_client(start_datetime, end_datetime, &pool)
            .await?;

        if !filenames.is_empty() {
            cli.process_filenames(&filenames).await?;
            cli.proc_everything().await?;
        }

        Ok(filenames)
    }
}
