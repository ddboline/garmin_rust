use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use fitbit_lib::fitbit_heartrate::import_garmin_json_file;
use garmin_lib::common::garmin_connect_activity::import_garmin_connect_activity_json_file;

#[derive(StructOpt, Debug, Clone)]
enum JsonImportOpts {
    #[structopt(alias = "act")]
    Activities {
        #[structopt(short = "f", long = "file")]
        filename: PathBuf,
    },
    #[structopt(alias = "hr")]
    Heartrate {
        #[structopt(short = "f", long = "files")]
        files: Vec<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let opts = JsonImportOpts::from_args();

    match opts {
        JsonImportOpts::Activities { filename } => {
            import_garmin_connect_activity_json_file(&filename).await?;
        }
        JsonImportOpts::Heartrate { files } => {
            for file in files {
                import_garmin_json_file(&file)?;
            }
        }
    }
    Ok(())
}
