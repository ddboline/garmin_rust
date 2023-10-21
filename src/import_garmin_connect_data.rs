#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

use anyhow::Error;
use clap::Parser;
use std::{collections::BTreeSet, path::PathBuf};
use tempdir::TempDir;

use fitbit_lib::fitbit_heartrate::{
    import_garmin_heartrate_file, import_garmin_json_file, FitbitHeartRate,
};
use garmin_lib::{
    common::{
        garmin_config::GarminConfig,
        garmin_connect_activity::import_garmin_connect_activity_json_file, pgpool::PgPool,
    },
    utils::garmin_util::extract_zip_from_garmin_connect_multiple,
};

#[derive(Parser, Debug, Clone)]
enum JsonImportOpts {
    #[clap(alias = "act")]
    Activities {
        #[clap(short = 'f', long = "file")]
        filename: PathBuf,
    },
    #[clap(alias = "hr")]
    Heartrate {
        #[clap(short = 'f', long = "files")]
        files: Vec<PathBuf>,
    },
    #[clap(alias = "hrs")]
    Heartrates {
        #[clap(short = 'f', long = "file")]
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = GarminConfig::get_config(None)?;
    let opts = JsonImportOpts::parse();
    let pool = PgPool::new(&config.pgurl);
    let mut dates = BTreeSet::new();

    match opts {
        JsonImportOpts::Activities { filename } => {
            import_garmin_connect_activity_json_file(&filename).await?;
        }
        JsonImportOpts::Heartrate { files } => {
            for file in files {
                import_garmin_json_file(&config, &file)?;
                if import_garmin_json_file(&config, &file).is_err() {
                    dates.extend(import_garmin_heartrate_file(&config, &file)?.into_iter());
                }
            }
        }
        JsonImportOpts::Heartrates { file } => {
            let tempdir = TempDir::new("garmin_zip")?;
            let ziptmpdir = tempdir.path();
            let files = extract_zip_from_garmin_connect_multiple(&file, ziptmpdir)?;
            for file in files {
                dates.extend(import_garmin_heartrate_file(&config, &file)?);
                println!("processed {}", file.to_string_lossy());
            }
        }
    }
    for date in dates {
        FitbitHeartRate::calculate_summary_statistics(&config, &pool, date).await?;
    }
    Ok(())
}
