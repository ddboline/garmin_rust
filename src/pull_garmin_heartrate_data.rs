use anyhow::Error;
use std::{env::args, path::Path};

use fitbit_lib::fitbit_heartrate::{import_garmin_heartrate_file, import_garmin_json_file};
use garmin_lib::common::garmin_connect_activity::import_garmin_connect_activity_json_file;

#[tokio::main]
async fn main() -> Result<(), Error> {
    for arg in args() {
        let filename = Path::new(&arg);
        if filename.exists() {
            let fname = arg.to_lowercase();
            if fname.ends_with("fit") {
                import_garmin_heartrate_file(&filename)?;
            } else if fname.ends_with("json") {
                if import_garmin_json_file(&filename).is_err() {
                    import_garmin_connect_activity_json_file(&filename).await?;
                }
            }
        }
    }
    Ok(())
}
