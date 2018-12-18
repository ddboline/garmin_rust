#![allow(clippy::needless_pass_by_value)]

extern crate garmin_rust;

use std::env::var;
use std::error::Error;

use subprocess::{Exec, Redirection};

use lambda_runtime::{error::HandlerError, lambda, Context};
use log::error;
use serde_derive::{Deserialize, Serialize};
use simple_logger;

use garmin_rust::garmin_config::GarminConfig;
use garmin_rust::garmin_summary::GarminSummary;
use garmin_rust::garmin_sync::GarminSync;

#[derive(Deserialize)]
struct CustomEvent {
    #[serde(rename = "fileName")]
    file_name: String,
}

#[derive(Serialize)]
struct CustomOutput {
    message: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    lambda!(my_handler);

    Ok(())
}

fn my_handler(event: CustomEvent, c: Context) -> Result<CustomOutput, HandlerError> {
    if event.file_name == "" {
        error!("Empty filename in request {}", c.aws_request_id);
        return Err(c.new_error("Empty filename"));
    }

    let config = GarminConfig::new().from_env();

    if GarminSync::new()
        .get_list_of_keys(&config.gps_bucket)
        .expect("Failed to grab keys")
        .iter()
        .filter(|(k, _, _)| *k == event.file_name)
        .count()
        == 0
    {
        error!(
            "Filename {} in request {} does not exist in gps bucket",
            event.file_name, c.aws_request_id
        );
        return Err(c.new_error("Empty filename"));
    }

    let command = match var("LAMBDA_TASK_ROOT") {
        Ok(x) => format!("{}/bin/fit2tcx --help", x),
        Err(_) => "fit2tcx --help".to_string(),
    };
    
    println!("command is {}", command);

    let result = Exec::shell(command)
        .stdout(Redirection::Pipe)
        .capture()
        .expect("Failed to capture stdout")
        .stdout_str();

    GarminSummary::process_and_upload_single_gps_file(
        &event.file_name,
        &config.gps_bucket,
        &config.cache_bucket,
        &config.summary_bucket,
    )
    .expect("Failed to process gps file");

    Ok(CustomOutput {
        message: format!("Processing, {}! {}", event.file_name, result),
    })
}
