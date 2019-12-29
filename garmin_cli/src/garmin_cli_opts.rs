use chrono::{Duration, Utc};
use failure::Error;
use std::fs::File;
use std::path::Path;
use structopt::StructOpt;

use fitbit_lib::fitbit_client::FitbitClient;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_cli::{GarminCli, GarminCliOptions};

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
    pub fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = match GarminCliOpts::from_args() {
            GarminCliOpts::Bootstrap => GarminCliOptions::Bootstrap,
            GarminCliOpts::Proc { filename } => GarminCliOptions::ImportFileNames(filename),
            GarminCliOpts::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(&config, &["year".to_string()])
                } else {
                    GarminCli::process_pattern(&config, &patterns)
                };
                return GarminCli::with_config()?.run_cli(&req.options, &req.constraints);
            }
            GarminCliOpts::Connect => GarminCliOptions::Connect,
            GarminCliOpts::Sync { md5sum } => GarminCliOptions::Sync(md5sum),
        };

        let cli = GarminCli {
            opts: Some(opts),
            ..GarminCli::with_config()?
        };

        if let Some(GarminCliOptions::Connect) = cli.opts {
            let client = FitbitClient::from_file(cli.config.clone())?;
            let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
            let results: Result<Vec<_>, Error> = client
                .get_tcx_urls(start_date)?
                .into_iter()
                .map(|(start_time, tcx_url)| {
                    let fname = format!(
                        "{}/{}.tcx",
                        cli.config.gps_dir,
                        start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string(),
                    );
                    if !Path::new(&fname).exists() {
                        client.download_tcx(&tcx_url, &mut File::create(&fname)?)?;
                        Ok(Some(fname))
                    } else {
                        Ok(None)
                    }
                })
                .filter_map(|x| x.transpose())
                .collect();
            results?;
        }

        cli.garmin_proc().map(|_| ())
    }
}
