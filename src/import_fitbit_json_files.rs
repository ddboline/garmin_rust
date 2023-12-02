#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![type_length_limit = "1059389"]

use clap::Parser;
use stack_string::StackString;

use fitbit_lib::fitbit_heartrate::import_fitbit_json_files;
use garmin_lib::garmin_config::GarminConfig;

#[derive(Parser, Debug, Clone)]
pub struct JsonImportOpts {
    #[clap(short = 'd', long = "directory")]
    pub directory: StackString,
}

fn main() {
    env_logger::init();
    let config = GarminConfig::get_config(None).unwrap();
    let opts = JsonImportOpts::parse();
    import_fitbit_json_files(&config, opts.directory.as_str()).unwrap();
}
