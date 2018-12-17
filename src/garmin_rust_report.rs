extern crate env_logger;

use garmin_rust::garmin_cli::GarminCli;

fn main() {
    env_logger::init();

    GarminCli::new()
        .with_config()
        .cli_garmin_report()
        .expect("cli_garmin_report failed");
}
