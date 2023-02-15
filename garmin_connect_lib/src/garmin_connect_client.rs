use anyhow::{format_err, Error};
use bytes::Bytes;
use fantoccini::{Client, Locator};
use http::Method;
use log::{debug, info};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use time::{Date, OffsetDateTime};
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
    pub last_used: OffsetDateTime,
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
    #[must_use]
    pub fn new(config: GarminConfig) -> Self {
        Self {
            config,
            client: None,
            webdriver: None,
            last_used: OffsetDateTime::now_utc(),
            display_name: None,
            trigger_auth: true,
        }
    }

    async fn raw_get(client: &mut Client, url: &Url) -> Result<Bytes, Error> {
        let mut retry_count = 0;
        let raw = loop {
            if let Ok(raw) = client.raw_client_for(Method::GET, url.as_str()).await {
                break raw;
            } else if retry_count < 5 {
                sleep(Duration::from_secs(5)).await;
                retry_count += 1;
            } else {
                return Err(format_err!("Retry failed"));
            }
        };
        hyper::body::to_bytes(raw.into_body())
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if authorize fails
    pub async fn authorize(&mut self) -> Result<(), Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        info!("begin authorize");
        client
            .goto(
                self.config
                    .garmin_connect_sso_endpoint
                    .as_ref()
                    .ok_or_else(|| format_err!("Bad URL"))?
                    .as_str(),
            )
            .await?;

        client
            .wait()
            .for_element(Locator::Id("gauth-widget-frame-gauth-widget"))
            .await?;

        client
            .find(Locator::Id("gauth-widget-frame-gauth-widget"))
            .await?
            .enter_frame()
            .await?;

        let form = client.form(Locator::Id("login-form")).await?;
        form.set_by_name("username", &self.config.garmin_connect_email)
            .await?
            .set_by_name("password", &self.config.garmin_connect_password)
            .await?;
        info!("begin login");
        sleep(Duration::from_secs(5)).await;
        client
            .find(Locator::XPath("//*[@name=\"rememberme\"]"))
            .await?
            .click()
            .await?;
        info!("click login");
        client
            .find(Locator::Id("login-btn-signin"))
            .await?
            .click()
            .await?;
        sleep(Duration::from_secs(5)).await;
        info!("after click");
        let modern_url = self
            .config
            .garmin_connect_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("/modern")?;
        info!("goto modern");
        client.goto(modern_url.as_str()).await?;

        client
            .wait()
            .at_most(Duration::from_secs(30))
            .for_element(Locator::XPath("//*[@class=\"main-header\"]"))
            .await?;
        info!("raw get");
        let js = Self::raw_get(client, &modern_url).await?;
        let text = std::str::from_utf8(&js)?;
        self.last_used = OffsetDateTime::now_utc();

        self.display_name
            .replace(GarminConnectClient::extract_display_name(text)?);
        if self.display_name.is_none() {
            self.trigger_auth = true;
        }
        Ok(())
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn close(&mut self) -> Result<(), Error> {
        if let Some(client) = self.client.take() {
            client.close().await?;
        }
        if let Some(mut webdriver) = self.webdriver.take() {
            if let Err(e) = webdriver.kill().await {
                debug!("Failed to kill {}", e);
            }
        }
        self.last_used = OffsetDateTime::now_utc();
        self.display_name.take();
        self.trigger_auth = true;
        Ok(())
    }

    /// # Errors
    /// Return error if api call fails
    pub fn extract_display_name(text: &str) -> Result<StackString, Error> {
        if let Some(line) = text
            .split('\n')
            .find(|x| x.contains("window.VIEWER_SOCIAL_PROFILE"))
        {
            #[derive(Deserialize)]
            struct SocialProfile {
                #[serde(rename = "displayName")]
                display_name: StackString,
            }

            let entry = line
                .split(" = ")
                .nth(1)
                .ok_or_else(|| format_err!("Unexpected format"))?
                .trim_matches(';');
            let val: SocialProfile = serde_json::from_str(entry)?;
            Ok(val.display_name)
        } else {
            Err(format_err!("NO DISPLAY NAME {text}"))
        }
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_heartrate(&mut self, date: Date) -> Result<GarminConnectHrData, Error> {
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
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("/proxy/wellness-service/wellness/dailyHeartRate/")?
            .join(display_name)?;
        let date_str = StackString::from_display(date);
        url.query_pairs_mut().append_pair("date", &date_str);
        let js = Self::raw_get(client, &url).await?;
        self.last_used = OffsetDateTime::now_utc();
        serde_json::from_slice(&js).map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
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
            let id_str = StackString::from_display(activity.activity_id);
            let fname = self
                .config
                .download_directory
                .join(&id_str)
                .with_extension("zip");
            let url = self
                .config
                .garmin_connect_api_endpoint
                .as_ref()
                .ok_or_else(|| format_err!("Bad URL"))?
                .join("/proxy/download-service/files/activity/")?
                .join(&id_str)?;
            let data = Self::raw_get(client, &url).await?;
            self.last_used = OffsetDateTime::now_utc();
            fs::write(&fname, &data).await?;
            filenames.push(fname);
        }
        Ok(filenames)
    }

    /// # Errors
    /// Return error if api call fails
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
    #[serde(alias = "userProfileId")]
    pub user_profile_id: u64,
    #[serde(alias = "totalKilocalories")]
    pub total_kilocalories: Option<f64>,
    #[serde(alias = "activeKilocalories")]
    pub active_kilocalories: Option<f64>,
    #[serde(alias = "bmrKilocalories")]
    pub bmr_kilocalories: Option<f64>,
    #[serde(alias = "totalSteps")]
    pub total_steps: Option<u64>,
    #[serde(alias = "totalDistanceMeters")]
    pub total_distance_meters: Option<u64>,
    #[serde(alias = "userDailySummaryId")]
    pub user_daily_summary_id: Option<u64>,
    #[serde(alias = "calendarDate")]
    pub calendar_date: Date,
}

/// # Errors
/// Return error if authorize fails
pub async fn check_version(cmd: &Path, prefix: &str) -> Result<u64, Error> {
    Command::new(cmd)
        .args(["--version"])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(Into::into)
        .and_then(|p| {
            if p.status.success() {
                if let Some(version) = String::from_utf8_lossy(&p.stdout).split(prefix).nth(1) {
                    if let Some(Ok(major)) = version.split('.').next().map(str::parse) {
                        return Ok(major);
                    }
                }
            }
            Err(format_err!("Version check failed"))
        })
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use log::debug;

    use garmin_lib::common::garmin_connect_activity::GarminConnectActivity;

    use crate::garmin_connect_client::GarminConnectClient;

    #[test]
    fn test_extract_display_name() -> Result<(), Error> {
        let resp_text = include_str!("../../tests/data/garmin_connect_display_name.html");
        let display_name = GarminConnectClient::extract_display_name(resp_text)?;
        assert_eq!(display_name.as_str(), "ddboline");
        Ok(())
    }

    #[test]
    fn test_activity_deserialization() -> Result<(), Error> {
        let s = include_str!("../tests/data/activity.json");
        let a: Vec<GarminConnectActivity> = serde_json::from_str(s)?;
        debug!("{a:?}");
        Ok(())
    }
}
