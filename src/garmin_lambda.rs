use anyhow::{format_err, Error};
use chrono::NaiveDate;
use serde::{Serialize, Deserialize};
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::garmin_config::GarminConfig;
use std::env::var;
use reqwest::Client;
use url::Url;

#[derive(Serialize, Deserialize, Clone)]
enum LambdaAction {
    All,
    HeartRate(NaiveDate),
    Activities,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomEvent {
    action: LambdaAction,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomOutput {
    message: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomError {
    error_message: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let aws_lambda_runtime_api = var("AWS_LAMBDA_RUNTIME_API")?;
    let url_base = Url::parse(&format!("https://{}/2018-06-01/", aws_lambda_runtime_api))?;
    let client = Client::builder().cookie_store(true).build()?;

    let url = url_base.join("/runtime/invocation/next")?;
    let response = client.get(url).send().await?;

    let request_id = response.headers()
        .get("Lambda-Runtime-Aws-Request-Id")
        .ok_or_else(|| format_err!("No request id"))?
        .to_str()?.to_string();
    let event: CustomEvent = response.json().await?;

    match handler(&client, event).await {
        Ok(output) => {
            let url = format!("/runtime/invocation/{}/response", request_id);
            let url = url_base.join(&url)?;
            client.post(url).json(&output).send().await?.error_for_status()?;
        },
        Err(e) => {
            let err = CustomError {
                error_message: e.to_string(),
            };
            let url = format!("/runtime/invocation/{}/error", request_id);
            let url = url_base.join(&url)?;
            client.post(url).json(&err).send().await?.error_for_status()?;
        },
    };
    Ok(())
}

async fn handler(client: &Client, event: CustomEvent) -> Result<CustomOutput, Error> {
    let config = GarminConfig::get_config(None)?;

    

    let mut connect = GarminConnectClient::new(config.clone());
    connect.init().await?;

    Ok(CustomOutput{message: "success".into()})
}

