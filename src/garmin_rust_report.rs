use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj};

fn main() {
    env_logger::init();

    GarminCliObj::with_config()
        .cli_garmin_report()
        .expect("cli_garmin_report failed");
}
