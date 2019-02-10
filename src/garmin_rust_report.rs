extern crate dotenv;
extern crate env_logger;

use garmin_rust::common::garmin_cli::{GarminCli, GarminCliObj};
use garmin_rust::common::garmin_correction_lap::GarminCorrectionList;
use garmin_rust::parsers::garmin_parse::GarminParse;

fn main() {
    env_logger::init();

    GarminCliObj::<GarminParse, GarminCorrectionList>::with_config()
        .cli_garmin_report()
        .expect("cli_garmin_report failed");
}
