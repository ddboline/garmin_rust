extern crate env_logger;

use garmin_rust::common::garmin_cli::GarminCli;

fn main() {
    env_logger::init();

    GarminCli::with_cli_proc()
        .garmin_proc()
        .expect("cli_garmin_proc failed");
}
