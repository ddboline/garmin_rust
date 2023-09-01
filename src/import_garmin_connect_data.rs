#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

use anyhow::Error;
use clap::Parser;
use std::path::PathBuf;

use fitbit_lib::fitbit_heartrate::{import_garmin_json_file, import_garmin_heartrate_file};
use garmin_lib::common::garmin_connect_activity::import_garmin_connect_activity_json_file;

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
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let opts = JsonImportOpts::parse();

    match opts {
        JsonImportOpts::Activities { filename } => {
            import_garmin_connect_activity_json_file(&filename).await?;
        }
        JsonImportOpts::Heartrate { files } => {
            for file in files {
                // import_garmin_json_file(&file)?;
                import_garmin_heartrate_file(&file)?;
            }
        }
    }
    Ok(())
}
