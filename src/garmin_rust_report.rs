use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj};

fn main() {
    env_logger::init();

    match GarminCliObj::with_config()
        .expect("config init failed")
        .cli_garmin_report()
    {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
