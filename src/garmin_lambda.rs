#![allow(clippy::needless_pass_by_value)]

use std::error::Error;

use subprocess::{Exec, Redirection};

use lambda_runtime::{error::HandlerError, lambda, Context};
use log::{self, error};
use serde_derive::{Deserialize, Serialize};
use simple_logger;

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

    let command = r#"fit2tcx --help"#;

    let result = Exec::shell(command)
        .stdout(Redirection::Pipe)
        .capture()
        .unwrap()
        .stdout_str();

    Ok(CustomOutput {
        message: format!("Processing, {}! {}", event.file_name, result),
    })
}
