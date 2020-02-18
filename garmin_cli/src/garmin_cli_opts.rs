use anyhow::Error;
use chrono::{Duration, Utc};
use std::fs::File;
use std::path::Path;
use structopt::StructOpt;
use tokio::task::spawn_blocking;

use fitbit_lib::fitbit_client::FitbitClient;
use garmin_lib::common::garmin_cli::{GarminCli, GarminCliOptions};
use garmin_lib::common::garmin_config::GarminConfig;

#[derive(StructOpt)]
pub enum GarminCliOpts {
    Bootstrap,
    Proc {
        #[structopt(short, long)]
        filename: Vec<String>,
    },
    Report {
        #[structopt(short, long)]
        patterns: Vec<String>,
    },
    Connect,
    Sync {
        #[structopt(short, long)]
        md5sum: bool,
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
                return GarminCli::with_config()?
                    .run_cli(&req.options, &req.constraints)
                    .await;
            }
            Self::Connect => GarminCliOptions::Connect,
            Self::Sync { md5sum } => GarminCliOptions::Sync(md5sum),
        };

        let cli = GarminCli {
            opts: Some(opts),
            ..GarminCli::with_config()?
        };

        if let Some(GarminCliOptions::Connect) = cli.opts {
            let config = cli.config.clone();
            let res: Result<_, Error> = spawn_blocking(move || {
                let client = FitbitClient::from_file(config.clone())?;
                let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
                for (start_time, tcx_url) in client.get_tcx_urls(start_date)? {
                    let fname = format!(
                        "{}/{}.tcx",
                        config.gps_dir,
                        start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string(),
                    );
                    if !Path::new(&fname).exists() {
                        client.download_tcx(&tcx_url, &mut File::create(&fname)?)?;
                    }
                }
                Ok(())
            })
            .await?;
            res?;
        }

        cli.garmin_proc().await.map(|_| ())
    }
}
