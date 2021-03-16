use anyhow::{format_err, Error};
use bytes::Bytes;
use chrono::{DateTime, NaiveDate, Utc};
use fantoccini::{Client, Locator, Method};
use log::debug;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::StackString;
use std::{path::PathBuf, process::Stdio};
use tokio::{
    fs,
    process::{Child, Command},
    time::sleep,
};

use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity, pgpool::PgPool,
};

use super::garmin_connect_hr_data::GarminConnectHrData;

pub struct GarminConnectClient {
    config: GarminConfig,
    client: Option<Client>,
    webdriver: Option<Child>,
    pub last_used: DateTime<Utc>,
    display_name: Option<StackString>,
    trigger_auth: bool,
}

impl Default for GarminConnectClient {
    fn default() -> Self {
        let config = GarminConfig::default();
        Self::new(config)
    }
}

impl GarminConnectClient {
    pub fn new(config: GarminConfig) -> Self {
        Self {
            config,
            client: None,
            webdriver: None,
            last_used: Utc::now(),
            display_name: None,
            trigger_auth: true,
        }
    }

    pub async fn init(&mut self) -> Result<(), Error> {
        if !self.config.webdriver_path.exists() {
            return Err(format_err!(
                "WEBDRIVER NOT FOUND {:?}",
                self.config.webdriver_path
            ));
        }
        if self.trigger_auth {
            let webdriver = Command::new(&self.config.webdriver_path)
                .args(&[&format!("--port={}", self.config.webdriver_port)])
                .kill_on_drop(true)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            self.webdriver.replace(webdriver);
            sleep(std::time::Duration::from_secs(5)).await;

            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({
                "args": ["--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage"],
                "binary":
                    &self.config.chrome_path.to_string_lossy()
            });
            caps.insert("goog:chromeOptions".to_string(), opts.clone());

            self.client.replace(
                Client::with_capabilities(
                    &format!("http://localhost:{}", self.config.webdriver_port),
                    caps,
                )
                .await?,
            );
            self.last_used = Utc::now();
            self.trigger_auth = false;
        }
        if self.display_name.is_none() {
            self.authorize().await?;
        }
        Ok(())
    }

    async fn raw_get(client: &mut Client, url: &Url) -> Result<Bytes, Error> {
        let raw = client.raw_client_for(Method::GET, url.as_str()).await?;
        hyper::body::to_bytes(raw.into_body())
            .await
            .map_err(Into::into)
    }

    pub async fn authorize(&mut self) -> Result<(), Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        client
            .goto(
                self.config
                    .garmin_connect_sso_endpoint
                    .as_ref()
                    .unwrap()
                    .as_str(),
            )
            .await?;
        let mut form = client.form(Locator::Id("login-form")).await?;
        form.set_by_name("username", &self.config.garmin_connect_email)
            .await?
            .set_by_name("password", &self.config.garmin_connect_password)
            .await?
            .submit()
            .await?;
        let modern_url = self.config.garmin_connect_api_endpoint.as_ref().unwrap();
        client.goto(modern_url.as_str()).await?;
        let js = Self::raw_get(client, &modern_url).await?;
        let text = std::str::from_utf8(&js)?;
        self.last_used = Utc::now();

        self.display_name
            .replace(GarminConnectClient::extract_display_name(text)?);
        if self.display_name.is_none() {
            self.trigger_auth = true;
        }
        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        if let Some(mut client) = self.client.take() {
            client.close().await?;
        }
        if let Some(mut webdriver) = self.webdriver.take() {
            if let Err(e) = webdriver.kill().await {
                debug!("Failed to kill {}", e);
            }
        }
        self.last_used = Utc::now();
        self.display_name.take();
        self.trigger_auth = true;
        Ok(())
    }

    pub fn extract_display_name(text: &str) -> Result<StackString, Error> {
        for entry in text.split('\n').filter(|x| x.contains("JSON.parse")) {
            let entry = entry.replace(r#"\""#, r#"""#).replace(r#"");"#, "");
            let entries: SmallVec<[&str; 2]> = entry.split(r#" = JSON.parse(""#).take(2).collect();
            if entries[0].contains("VIEWER_SOCIAL_PROFILE") {
                #[derive(Deserialize)]
                struct SocialProfile {
                    #[serde(rename = "displayName")]
                    display_name: StackString,
                }
                let val: SocialProfile = serde_json::from_str(entries[1])?;
                return Ok(val.display_name);
            }
        }
        Err(format_err!("NO DISPLAY NAME"))
    }

    pub async fn get_user_summary(
        &mut self,
        date: NaiveDate,
    ) -> Result<GarminConnectUserDailySummary, Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        let display_name = self
            .display_name
            .as_ref()
            .ok_or_else(|| format_err!("No display name"))?;
        let mut url = self
            .config
            .garmin_connect_api_endpoint
            .as_ref()
            .unwrap()
            .join("/proxy/usersummary-service/usersummary/daily/")?
            .join(display_name)?;
        url.query_pairs_mut()
            .append_pair("calendarDate", &date.to_string());
        let js = Self::raw_get(client, &url).await?;
        let user_summary: GarminConnectUserDailySummary = serde_json::from_slice(&js)?;
        self.last_used = Utc::now();
        if user_summary.total_steps.is_none() {
            self.trigger_auth = true;
        }
        Ok(user_summary)
    }

    pub async fn get_heartrate(&mut self, date: NaiveDate) -> Result<GarminConnectHrData, Error> {
        let display_name = self
            .display_name
            .as_ref()
            .ok_or_else(|| format_err!("No display name"))?;
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        let mut url = self
            .config
            .garmin_connect_api_endpoint
            .as_ref()
            .unwrap()
            .join("/proxy/wellness-service/wellness/dailyHeartRate/")?
            .join(display_name)?;
        url.query_pairs_mut().append_pair("date", &date.to_string());
        let js = Self::raw_get(client, &url).await?;
        self.last_used = Utc::now();
        serde_json::from_slice(&js).map_err(Into::into)
    }

    pub async fn get_activities(
        &mut self,
        _: DateTime<Utc>,
    ) -> Result<Vec<GarminConnectActivity>, Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        let url = self
            .config
            .garmin_connect_api_endpoint
            .as_ref()
            .unwrap()
            .join("/proxy/activitylist-service/activities/search/activities")?;
        let js = Self::raw_get(client, &url).await?;
        self.last_used = Utc::now();
        serde_json::from_slice(&js).map_err(Into::into)
    }

    pub async fn get_activity_files(
        &mut self,
        activities: &[GarminConnectActivity],
    ) -> Result<Vec<PathBuf>, Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        let mut filenames = Vec::new();

        fs::create_dir_all(&self.config.download_directory).await?;

        for activity in activities {
            let fname = self
                .config
                .download_directory
                .join(activity.activity_id.to_string())
                .with_extension("zip");
            let url = self
                .config
                .garmin_connect_api_endpoint
                .as_ref()
                .unwrap()
                .join("/proxy/download-service/files/activity/")?
                .join(&activity.activity_id.to_string())?;
            let data = Self::raw_get(client, &url).await?;
            self.last_used = Utc::now();
            fs::write(&fname, &data).await?;
            filenames.push(fname);
        }
        Ok(filenames)
    }

    pub async fn get_and_merge_activity_files(
        &mut self,
        activities: Vec<GarminConnectActivity>,
        pool: &PgPool,
    ) -> Result<Vec<PathBuf>, Error> {
        let activities = GarminConnectActivity::merge_new_activities(activities, pool).await?;
        self.get_activity_files(&activities).await
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GarminConnectUserDailySummary {
    #[serde(rename = "userProfileId")]
    pub user_profile_id: u64,
    #[serde(rename = "totalKilocalories")]
    pub total_kilocalories: Option<f64>,
    #[serde(rename = "activeKilocalories")]
    pub active_kilocalories: Option<f64>,
    #[serde(rename = "bmrKilocalories")]
    pub bmr_kilocalories: Option<f64>,
    #[serde(rename = "totalSteps")]
    pub total_steps: Option<u64>,
    #[serde(rename = "totalDistanceMeters")]
    pub total_distance_meters: Option<u64>,
    #[serde(rename = "userDailySummaryId")]
    pub user_daily_summary_id: Option<u64>,
    #[serde(rename = "calendarDate")]
    pub calendar_date: NaiveDate,
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Duration, Utc};
    use futures::future::try_join_all;
    use std::collections::HashMap;

    use garmin_lib::common::{
        garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity, pgpool::PgPool,
    };

    use crate::garmin_connect_client::GarminConnectClient;

    #[test]
    fn test_extract_display_name() -> Result<(), Error> {
        let resp_text = include_str!("../../tests/data/garmin_connect_display_name.html");
        let display_name = GarminConnectClient::extract_display_name(resp_text)?;
        assert_eq!(display_name.as_str(), "ddboline");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_proxy_get_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let mut session = GarminConnectClient::new(config.clone());
        session.init().await?;

        let user_summary = session
            .get_user_summary((Utc::now() - Duration::days(1)).naive_local().date())
            .await?;
        assert_eq!(user_summary.user_profile_id, 1377808);

        let max_timestamp = Utc::now() - Duration::days(14);
        let result = match session.get_activities(max_timestamp).await {
            Ok(r) => r,
            Err(_) => {
                println!("try reauth");
                session.authorize().await?;
                session.get_activities(max_timestamp).await?
            }
        };
        assert!(result.len() > 0);

        let config = GarminConfig::get_config(None)?;

        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = GarminConnectActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.activity_id, activity))
            .collect();

        let max_timestamp = Utc::now() - Duration::days(30);
        let new_activities: Vec<_> = session
            .get_activities(max_timestamp)
            .await?
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.activity_id))
            .collect();
        println!("{:?}", new_activities);
        let futures = new_activities.iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        assert_eq!(new_activities.len(), 0);
        Ok(())
    }
}
