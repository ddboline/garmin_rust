use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj};

fn main() {
    env_logger::init();

    GarminCliObj::with_cli_proc()
        .expect("config init failed")
        .garmin_proc()
        .expect("cli_garmin_proc failed");
}
