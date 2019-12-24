use structopt::StructOpt;
use failure::Error;

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
        let opts = match GarminCliOpts::from_args() {
            GarminCliOpts::Bootstrap => GarminCliOptions::Bootstrap,
            GarminCliOpts::Proc { filename } => GarminCliOptions::ImportFileNames(filename),
            GarminCliOpts::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(&["year".to_string()])
                } else {
                    GarminCli::process_pattern(&patterns)
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

        cli.garmin_proc().map(|_| ())
    }
}
