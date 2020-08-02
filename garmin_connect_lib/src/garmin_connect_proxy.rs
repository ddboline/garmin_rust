use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, Utc};
use fantoccini::{Client, Locator};
use log::debug;
use stack_string::StackString;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::fs;
use tokio::process::{Child, Command};
use url::Url;

use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
};

use super::garmin_connect_client::{
    GarminConnectClient, GarminConnectHrData, GarminConnectUserDailySummary,
};

const MODERN_URL: &str = "https://connect.garmin.com/modern";

pub struct GarminConnectProxy {
    config: GarminConfig,
    client: Option<Client>,
    webdriver: Option<Child>,
    pub last_used: DateTime<Utc>,
    display_name: Option<StackString>,
}

impl Default for GarminConnectProxy {
    fn default() -> Self {
        let config = GarminConfig::default();
        Self::new(config)
    }
}

impl GarminConnectProxy {
    pub fn new(config: GarminConfig) -> Self {
        Self {
            config,
            client: None,
            webdriver: None,
            last_used: Utc::now(),
            display_name: None,
        }
    }

    pub async fn init(&mut self, config: GarminConfig) -> Result<(), Error> {
        self.config = config;
        if self.webdriver.is_none() {
            if self.config.webdriver_path.exists() {
                let webdriver = Command::new(&self.config.webdriver_path)
                    .args(&[&format!("--port={}", self.config.webdriver_port)])
                    .kill_on_drop(true)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                self.webdriver.replace(webdriver);
            }
        }
        if self.client.is_none() {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({
                "args": ["--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage"],
                "binary":
                    "/usr/bin/google-chrome"
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
        }
        if self.display_name.is_none() {
            self.authorize().await?;
        }
        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        if let Some(mut webdriver) = self.webdriver.take() {
            if let Err(e) = webdriver.kill() {
                debug!("Failed to kill {}", e);
            }
        }
        if let Some(mut client) = self.client.take() {
            client.close().await?;
        }
        self.last_used = Utc::now();
        Ok(())
    }

    pub async fn authorize(&mut self) -> Result<(), Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        client.goto("https://sso.garmin.com/sso/signin").await?;
        let mut form = client.form(Locator::Id("login-form")).await?;
        form.set_by_name("username", &self.config.garmin_connect_email)
            .await?
            .set_by_name("password", &self.config.garmin_connect_password)
            .await?
            .submit()
            .await?;

        client.goto(MODERN_URL).await?;
        let raw = client
            .raw_client_for(fantoccini::Method::GET, MODERN_URL)
            .await?;
        self.last_used = Utc::now();
        let js = hyper::body::to_bytes(raw.into_body()).await?;
        let text = std::str::from_utf8(&js)?;

        self.display_name
            .replace(GarminConnectClient::extract_display_name(text)?);

        Ok(())
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
        let url_prefix = format!(
            "{}/proxy/usersummary-service/usersummary/daily/{}",
            MODERN_URL, display_name,
        );
        let url = Url::parse_with_params(&url_prefix, &[("calendarDate", &date.to_string())])?;
        let raw = client
            .raw_client_for(fantoccini::Method::GET, url.as_str())
            .await?;
        self.last_used = Utc::now();
        let js = hyper::body::to_bytes(raw.into_body()).await?;
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
        let url = format!(
            "{}/proxy/activitylist-service/activities/search/activities",
            MODERN_URL
        );
        let raw = client.raw_client_for(fantoccini::Method::GET, &url).await?;
        self.last_used = Utc::now();
        let js = hyper::body::to_bytes(raw.into_body()).await?;
        serde_json::from_slice(&js).map_err(Into::into)
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
        let url = format!(
            "{}/proxy/wellness-service/wellness/dailyHeartRate/{}",
            MODERN_URL, display_name
        );
        let url = Url::parse_with_params(&url, &[("date", &date.to_string())])?;
        let raw = client
            .raw_client_for(fantoccini::Method::GET, url.as_str())
            .await?;
        self.last_used = Utc::now();
        let js = hyper::body::to_bytes(raw.into_body()).await?;
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
        for activity in activities {
            let fname = self
                .config
                .home_dir
                .join("Downloads")
                .join(activity.activity_id.to_string())
                .with_extension("zip");
            let url = format!(
                "{}/proxy/download-service/files/activity/{}",
                MODERN_URL, activity.activity_id
            );
            let raw = client
                .raw_client_for(fantoccini::Method::GET, url.as_str())
                .await?;
            self.last_used = Utc::now();
            let data = hyper::body::to_bytes(raw.into_body()).await?;
            fs::write(&fname, &data).await?;
            filenames.push(fname);
        }
        Ok(filenames)
    }
}
