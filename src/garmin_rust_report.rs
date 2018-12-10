extern crate env_logger;

use garmin_rust::garmin_cli;

fn main() {
    env_logger::init();

    garmin_cli::cli_garmin_report().unwrap();
}
