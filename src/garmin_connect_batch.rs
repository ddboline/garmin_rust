#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::used_underscore_binding)]

use anyhow::{format_err, Error};
use chrono::{Duration, Utc};
use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
};
use log::debug;
use maplit::hashmap;
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde::Deserialize;
use std::collections::HashSet;
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Error> {
    #[derive(Deserialize, Debug)]
    struct LoggedUser {
        email: String,
    }

    env_logger::init();

    let client = Client::builder().cookie_store(true).build()?;

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
        .post(url)
        .json(&data)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    debug!("Logged in {:?}", user);

    let mut connect = GarminConnectClient::new(config.clone());
    connect.init().await?;

    for idx in 0..3 {
        let date = (Utc::now() - Duration::days(idx)).naive_utc().date();
        let hr_values = connect.get_heartrate(date).await?;
        let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
        if !hr_values.is_empty() {
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
    }

    let connect_activities = connect
        .get_activities(Some(Utc::now() - Duration::days(14)))
        .await?;
    let url = remote_url.join("/garmin/garmin_connect_activities_db")?;
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
    println!("new activities {:?}", new_activities);
    if let Ok(filenames) = connect.get_activity_files(&new_activities).await {
        if !filenames.is_empty() {
            for filename in &filenames {
                let dname = filename
                    .file_name()
                    .ok_or_else(|| format_err!("no filename"))?
                    .to_string_lossy();
                let fname = filename.to_string_lossy().to_string();
                let url = remote_url.join("/garmin/upload_file")?;
                let url = Url::parse_with_params(url.as_str(), &[("filename", dname)])?;
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
        println!("Processed {:?}", filenames);
    }
    Ok(())
}
