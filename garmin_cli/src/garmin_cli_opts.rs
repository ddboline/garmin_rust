use anyhow::Error;
use chrono::{Duration, Utc};
use futures::future::try_join_all;
use stack_string::StackString;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::{
    fs::{read_to_string, File},
    io::{stdin, stdout, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    try_join,
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

#[derive(StructOpt)]
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
    Import {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities', 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[structopt(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities', 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
}

impl GarminCliOpts {
    pub async fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = match Self::from_args() {
            Self::Bootstrap => GarminCliOptions::Bootstrap,
            Self::Proc { filename } => GarminCliOptions::ImportFileNames(filename),
            Self::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(&config, &["year".to_string()])
                } else {
                    GarminCli::process_pattern(&config, &patterns)
                };
                let cli = GarminCli::with_config()?;
                let stdout_task = cli.stdout.spawn_stdout_task();
                cli.run_cli(&req.options, &req.constraints).await?;
                cli.stdout.close().await?;
                return stdout_task.await?;
            }
            Self::Connect => GarminCliOptions::Connect,
            Self::Sync { md5sum } => GarminCliOptions::Sync(md5sum),
            Self::Fitbit { all } => {
                let today = Utc::now().naive_local().date();

                let cli = GarminCli::with_config()?;
                let stdout_task = cli.stdout.spawn_stdout_task();
                let config = GarminConfig::get_config(None)?;
                let pool = PgPool::new(&config.pgurl);
                let client = FitbitClient::with_auth(config.clone()).await?;
                let (updates, _) = try_join!(
                    client.sync_everything(&pool),
                    client.import_fitbit_heartrate(today)
                )?;
                cli.stdout.send(format!("{:?}", updates).into())?;

                let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
                let filenames: Vec<_> = client
                    .sync_tcx(start_date)
                    .await?
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                cli.stdout.send(filenames.join("\n").into())?;

                cli.stdout.close().await?;
                FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, today).await?;

                if all {
                    FitbitHeartRate::get_all_summary_statistics(&client.config, &pool).await?;
                }
                return stdout_task.await?;
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
                    }
                    "strava_activities" => {
                        let activities: Vec<StravaActivity> = serde_json::from_str(&data)?;
                        StravaActivity::upsert_activities(&activities, &pool).await?;
                    }
                    "fitbit_activities" => {
                        let activities: Vec<FitbitActivity> = serde_json::from_str(&data)?;
                        FitbitActivity::upsert_activities(&activities, &pool).await?;
                    }
                    "garmin_connect_activities" => {
                        let activities: Vec<GarminConnectActivity> = serde_json::from_str(&data)?;
                        GarminConnectActivity::upsert_activities(&activities, &pool).await?;
                    }
                    "race_results" => {
                        let results: Vec<RaceResults> = serde_json::from_str(&data)?;
                        let futures = results.into_iter().map(|result| {
                            let pool = pool.clone();
                            async move { result.update_db(&pool).await.map_err(Into::into) }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        results?;
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
                        for measurement in
                            ScaleMeasurement::read_from_db(&pool, Some(start_date), None).await?
                        {
                            file.write_all(&serde_json::to_vec(&measurement)?).await?;
                        }
                    }
                    "strava_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        for activity in
                            StravaActivity::read_from_db(&pool, Some(start_date), None).await?
                        {
                            file.write_all(&serde_json::to_vec(&activity)?).await?;
                        }
                    }
                    "fitbit_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        for activity in
                            FitbitActivity::read_from_db(&pool, Some(start_date), None).await?
                        {
                            file.write_all(&serde_json::to_vec(&activity)?).await?;
                        }
                    }
                    "garmin_connect_activities" => {
                        let start_date = (Utc::now() - Duration::days(7)).naive_local().date();
                        for activity in
                            GarminConnectActivity::read_from_db(&pool, Some(start_date), None)
                                .await?
                        {
                            file.write_all(&serde_json::to_vec(&activity)?).await?;
                        }
                    }
                    "race_results" => {
                        for result in
                            RaceResults::get_results_by_type(RaceType::Personal, &pool).await?
                        {
                            file.write_all(&serde_json::to_vec(&result)?).await?;
                        }
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

        let stdout_task = cli.stdout.spawn_stdout_task();

        Self::garmin_proc(&cli).await?;
        cli.stdout.close().await?;
        stdout_task.await?
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
        cli.stdout.send(results.join("\n").into())?;
        Ok(())
    }

    pub async fn sync_with_garmin_connect(cli: &GarminCli) -> Result<Vec<PathBuf>, Error> {
        if let Some(max_datetime) = get_maximum_begin_datetime(&cli.pool).await? {
            let mut session = GarminConnectClient::default();
            session.init(cli.config.clone()).await?;

            let activities = session.get_activities(max_datetime).await?;

            let activities =
                GarminConnectActivity::merge_new_activities(activities, &cli.pool).await?;

            session
                .get_heartrate((Utc::now()).naive_local().date())
                .await?;

            if let Ok(filenames) = session.get_activity_files(&activities).await {
                if !filenames.is_empty() {
                    cli.process_filenames(&filenames).await?;
                    cli.proc_everything().await?;
                }
                return Ok(filenames);
            }
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
