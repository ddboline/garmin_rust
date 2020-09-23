#![type_length_limit = "1059504"]

use garmin_cli::garmin_cli_opts::GarminCliOpts;

#[tokio::main]
async fn main() {
    env_logger::init();

    match GarminCliOpts::process_args().await {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
