use anyhow::Error;
use chrono::{Duration, Utc};
use stack_string::StackString;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::try_join;

use fitbit_lib::{fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate};
use garmin_connect_lib::garmin_connect_client::get_garmin_connect_session;
use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_summary::get_maximum_begin_datetime, pgpool::PgPool,
};

use crate::garmin_cli::{GarminCli, GarminCliOptions};

#[derive(StructOpt)]
pub enum GarminCliOpts {
    Bootstrap,
    Proc {
        #[structopt(short, long)]
        filename: Vec<PathBuf>,
    },
    Report {
        #[structopt(short, long)]
        patterns: Vec<StackString>,
    },
    Connect,
    Sync {
        #[structopt(short, long)]
        md5sum: bool,
    },
    Fitbit,
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
            Self::Fitbit => {
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
            for (start_time, tcx_url) in client.get_tcx_urls(start_date).await? {
                let fname = config
                    .gps_dir
                    .join(start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string())
                    .with_extension("tcx");
                if !fname.exists() {
                    let data = client.download_tcx(&tcx_url).await?;
                    tokio::fs::write(&fname, &data).await?;
                }
            }
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
            let session = get_garmin_connect_session(&cli.config).await?;
            let activities = session.get_activities(max_datetime).await?;
            let filenames = session.get_activity_files(&activities).await?;
            session
                .get_heartrate((Utc::now()).naive_local().date())
                .await?;
            cli.process_filenames(&filenames).await?;
            return Ok(filenames);
        }
        Ok(Vec::new())
    }
}
