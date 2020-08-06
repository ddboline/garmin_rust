use anyhow::Error;
use chrono::{Duration, Utc};
use stack_string::StackString;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::try_join;

use fitbit_lib::{fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate};
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
    garmin_summary::get_maximum_begin_datetime, pgpool::PgPool,
};
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
                cli.stdout.close().await?;
                FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, today).await?;

                if all {
                    FitbitHeartRate::get_all_summary_statistics(&client.config, &pool).await?;
                }
                return stdout_task.await?;
            }
        };

        let cli = GarminCli {
            opts: Some(opts),
            ..GarminCli::with_config()?
        };

        let stdout_task = cli.stdout.spawn_stdout_task();

        if let Some(GarminCliOptions::Connect) = cli.opts {
            let config = cli.config.clone();
            let client = FitbitClient::with_auth(config.clone()).await?;
            let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
            let filenames: Vec<_> = client
                .sync_tcx(start_date)
                .await?
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            cli.stdout.send(filenames.join("\n").into())?;
        }

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
        let max_datetime = Utc::now() - Duration::days(15);

        let client = StravaClient::with_auth(config).await?;
        let filenames = client.sync_with_client(max_datetime, &pool).await?;

        if !filenames.is_empty() {
            cli.process_filenames(&filenames).await?;
            cli.proc_everything().await?;
        }

        Ok(filenames)
    }
}
