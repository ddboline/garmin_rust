use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj};

fn main() {
    env_logger::init();

    match GarminCliObj::with_cli_proc()
        .expect("config init failed")
        .garmin_proc()
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
