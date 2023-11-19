#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![type_length_limit = "1161159"]

use garmin_cli::garmin_cli_opts::GarminCliOpts;

#[tokio::main]
async fn main() {
    env_logger::init();

    match GarminCliOpts::process_args().await {
        Ok(()) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
}
