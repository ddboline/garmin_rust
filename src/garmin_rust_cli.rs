use garmin_cli::garmin_cli_opts::GarminCliOpts;

fn main() {
    env_logger::init();

    match GarminCliOpts::process_args() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
