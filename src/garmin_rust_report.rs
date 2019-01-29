extern crate dotenv;
extern crate env_logger;

use garmin_rust::common::garmin_cli::GarminCli;

fn main() {
    env_logger::init();

    GarminCli::with_config()
        .cli_garmin_report()
        .expect("cli_garmin_report failed");
}
