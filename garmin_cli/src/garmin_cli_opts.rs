use anyhow::Error;
use chrono::{Duration, Utc};
use std::path::PathBuf;
use structopt::StructOpt;

use fitbit_lib::{fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate};
use garmin_lib::{
    common::{
        garmin_cli::{GarminCli, GarminCliOptions},
        garmin_config::GarminConfig,
        pgpool::PgPool,
    },
    utils::stack_string::StackString,
};

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
                let config = GarminConfig::get_config(None)?;
                let pool = PgPool::new(&config.pgurl);
                FitbitHeartRate::get_all_summary_statistics(&config, &pool).await?;
                return Ok(());
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
                let fname = config.gps_dir.join(format!(
                    "{}.tcx",
                    start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string()
                ));
                if !fname.exists() {
                    let data = client.download_tcx(&tcx_url).await?;
                    tokio::fs::write(&fname, &data).await?;
                }
            }
        }

        cli.garmin_proc().await?;
        cli.stdout.close().await?;
        stdout_task.await?
    }
}
