extern crate env_logger;

use garmin_rust::garmin_cli;

fn main() {
    env_logger::init();

    garmin_cli::cli_garmin_proc().expect("cli_garmin_proc failed");
}
