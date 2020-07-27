use anyhow::Error;
use std::{env::args, path::Path};

use fitbit_lib::fitbit_heartrate::import_garmin_heartrate_file;

fn main() -> Result<(), Error> {
    for arg in args() {
        let filename = Path::new(&arg);
        if filename.exists() && arg.to_lowercase().ends_with("fit") {
            import_garmin_heartrate_file(&filename)?;
        }
    }
    Ok(())
}
