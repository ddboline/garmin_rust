use anyhow::{format_err, Error};
use chrono::{Duration, NaiveDate, Utc};
use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
};
use log::debug;
use maplit::hashmap;
use reqwest::{
    header::HeaderMap,
    multipart::{Form, Part},
    Client,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, env::var};
use url::Url;

#[derive(Serialize, Deserialize, Clone, Copy)]
enum LambdaAction {
    All(NaiveDate),
    HeartRate(NaiveDate),
    Activities,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct CustomEvent {
    action: LambdaAction,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomOutput {
    message: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomError {
    #[serde(rename = "errorMessage")]
    error_message: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let aws_lambda_runtime_api = var("AWS_LAMBDA_RUNTIME_API")?;
    let url_base = Url::parse(&format!("https://{}/2018-06-01/", aws_lambda_runtime_api))?;

    let client = Client::builder().cookie_store(true).build()?;

    if let Err(e) = get_next(&client, &url_base).await {
        let url = url_base.join("/runtime/init/error")?;
        let err = CustomError {
            error_message: e.to_string(),
        };
        let mut header = HeaderMap::new();
        header.insert("Lambda-Runtime-Function-Error-Type", "Unhandled".parse()?);
        client
            .post(url)
            .headers(header)
            .json(&err)
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(())
}

async fn get_next(client: &Client, url_base: &Url) -> Result<(), Error> {
    let url = url_base.join("/runtime/invocation/next")?;
    let response = client.get(url).send().await?;

    let request_id = response
        .headers()
        .get("Lambda-Runtime-Aws-Request-Id")
        .ok_or_else(|| format_err!("No request id"))?
        .to_str()?
        .to_string();
    let event: CustomEvent = response.json().await?;

    match handler(&client, event).await {
        Ok(output) => {
            let url = format!("/runtime/invocation/{}/response", request_id);
            let url = url_base.join(&url)?;
            client
                .post(url)
                .json(&output)
                .send()
                .await?
                .error_for_status()?;
        }
        Err(e) => {
            let err = CustomError {
                error_message: e.to_string(),
            };
            let url = format!("/runtime/invocation/{}/error", request_id);
            let url = url_base.join(&url)?;
            client
                .post(url)
                .json(&err)
                .send()
                .await?
                .error_for_status()?;
        }
    };
    Ok(())
}

async fn handler(client: &Client, event: CustomEvent) -> Result<CustomOutput, Error> {
    #[derive(Deserialize, Debug)]
    struct LoggedUser {
        email: String,
    }

    let config = GarminConfig::get_config(None)?;

    let remote_url = config
        .remote_url
        .as_ref()
        .ok_or_else(|| format_err!("No remote_url given"))?;
    let remote_email = config
        .remote_email
        .as_ref()
        .ok_or_else(|| format_err!("No remote email given"))?;
    let remote_password = config
        .remote_password
        .as_ref()
        .ok_or_else(|| format_err!("No remote password given"))?;

    let data = hashmap! {
        "email" => remote_email.as_str(),
        "password" => remote_password.as_str(),
    };

    let url = remote_url.join("api/auth")?;
    let user: LoggedUser = client
        .get(url)
        .json(&data)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    debug!("Logged in {:?}", user);

    let mut connect = GarminConnectClient::new(config.clone());
    connect.init().await?;

    let hr_date = match event.action {
        LambdaAction::All(d) => Some(d),
        LambdaAction::HeartRate(d) => Some(d),
        LambdaAction::Activities => None,
    };

    if let Some(date) = hr_date {
        let hr_values = connect.get_heartrate(date).await?;
        let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
        let data = hashmap! {
            "updates" => hr_values,
        };
        let url = remote_url.join("/garmin/fitbit/heartrate_cache")?;
        client
            .post(url)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
    }

    if let LambdaAction::HeartRate(_) = event.action {
        Ok(CustomOutput {
            message: "Success".into(),
        })
    } else {
        let date = Utc::now() - Duration::days(30);
        let connect_activities = connect.get_activities(date).await?;

        let date = (Utc::now() - Duration::days(60)).naive_utc().date();
        let url = Url::parse_with_params(
            remote_url
                .join("/garmin/garmin_connect_activities_db")?
                .as_str(),
            &[("start_date", date.to_string())],
        )?;
        let db_activities: Vec<GarminConnectActivity> = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let db_set: HashSet<i64> = db_activities.iter().map(|a| a.activity_id).collect();
        let new_activities: Vec<_> = connect_activities
            .into_iter()
            .filter(|a| !db_set.contains(&a.activity_id))
            .collect();
        if let Ok(filenames) = connect.get_activity_files(&new_activities).await {
            if !filenames.is_empty() {
                for filename in &filenames {
                    let fname = filename.to_string_lossy().to_string();
                    let url = remote_url.join("/garmin/upload_file")?;
                    let part = Part::bytes(tokio::fs::read(filename).await?).file_name(fname);
                    let form = Form::new().part("file", part);
                    client
                        .post(url)
                        .multipart(form)
                        .send()
                        .await?
                        .error_for_status()?;
                }
            }
            return Ok(CustomOutput {
                message: format!("Processed {:?}", filenames),
            });
        }
        Ok(CustomOutput {
            message: "Success".into(),
        })
    }
}
