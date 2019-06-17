use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj};

fn main() {
    env_logger::init();

    GarminCliObj::with_config()
        .expect("config init failed")
        .cli_garmin_report()
        .expect("cli_garmin_report failed");
}
