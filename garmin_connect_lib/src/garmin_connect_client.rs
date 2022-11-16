use anyhow::{format_err, Error};
use bytes::Bytes;
use fantoccini::{Client, ClientBuilder, Locator};
use http::Method;
use log::{debug, info};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use time::{Date, OffsetDateTime};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};
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

    /// # Errors
    /// Return error if spawn fails
    pub async fn init(&mut self) -> Result<(), Error> {
        if !self.config.webdriver_path.exists() {
            return Err(format_err!(
                "WEBDRIVER NOT FOUND {:?}",
                self.config.webdriver_path
            ));
        }
        if !self.config.chrome_path.exists() {
            return Err(format_err!(
                "CHROME NOT FOUND {:?}",
                self.config.chrome_path
            ));
        }
        let chrome_version = check_version(&self.config.chrome_path, "Google Chrome ").await?;
        let driver_version = check_version(&self.config.webdriver_path, "ChromeDriver ").await?;
        if chrome_version != driver_version {
            return Err(format_err!(
                "Chrome version {chrome_version} does not match driver version {driver_version}"
            ));
        }
        if self.trigger_auth {
            let webdriver = Command::new(&self.config.webdriver_path)
                .args([&format_sstr!("--port={}", self.config.webdriver_port)])
                .kill_on_drop(true)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            self.webdriver.replace(webdriver);
            sleep(Duration::from_secs(5)).await;

            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({
                "args": [
                    "--disable-gpu",
                    "--no-sandbox",
                    "--disable-dev-shm-usage"
                ],
                "binary":
                    &self.config.chrome_path.to_string_lossy()
            });
            caps.insert("goog:chromeOptions".to_string(), opts.clone());
            caps.insert("pageLoadStrategy".to_string(), "eager".into());
            caps.insert("unhandledPromptBehavior".to_string(), "accept".into());
            let client = ClientBuilder::rustls()
                .capabilities(caps)
                .connect(&format_sstr!(
                    "http://localhost:{}",
                    self.config.webdriver_port
                ))
                .await?;
            client
                .set_ua(
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                     Chrome/90.0.4430.212 Safari/537.36",
                )
                .await?;

            self.client.replace(client);
            self.last_used = OffsetDateTime::now_utc();
            self.trigger_auth = false;
        }
        if self.display_name.is_none() {
            self.authorize().await?;
        }
        Ok(())
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
            .ok_or_else(|| format_err!("Bad URL"))?;
        info!("goto modern");
        client.goto(modern_url.as_str()).await?;

        client
            .wait()
            .at_most(Duration::from_secs(30))
            .for_element(Locator::XPath("//*[@class=\"main-header\"]"))
            .await?;
        info!("raw get");
        let js = Self::raw_get(client, modern_url).await?;
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
    pub async fn get_user_summary(
        &mut self,
        date: Date,
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
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("/proxy/usersummary-service/usersummary/daily/")?
            .join(display_name)?;
        let date_str = StackString::from_display(date);
        url.query_pairs_mut().append_pair("calendarDate", &date_str);
        let js = Self::raw_get(client, &url).await?;
        let user_summary: GarminConnectUserDailySummary = serde_json::from_slice(&js)?;
        self.last_used = OffsetDateTime::now_utc();
        if user_summary.total_steps.is_none() {
            self.trigger_auth = true;
        }
        Ok(user_summary)
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
    pub async fn get_activities(
        &mut self,
        start_datetime: Option<OffsetDateTime>,
    ) -> Result<Vec<GarminConnectActivity>, Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format_err!("No client"))?;
        let mut url = self
            .config
            .garmin_connect_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("/proxy/activitylist-service/activities/search/activities")?;
        if let Some(start_datetime) = start_datetime {
            let datetime_str = StackString::from_display(start_datetime.to_timezone(UTC).date());
            url.query_pairs_mut()
                .append_pair("startDate", &datetime_str);
        }
        info!("raw get activities");
        let js = Self::raw_get(client, &url).await?;
        info!("raw got activities");
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

async fn check_version(cmd: &Path, prefix: &str) -> Result<u64, Error> {
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
    use futures::{future::try_join_all, TryStreamExt};
    use log::debug;
    use std::collections::HashMap;
    use time::{Duration, OffsetDateTime};
    use time_tz::OffsetDateTimeExt;

    use garmin_lib::{
        common::{
            garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
            pgpool::PgPool,
        },
        utils::date_time_wrapper::DateTimeWrapper,
    };

    use crate::garmin_connect_client::{check_version, GarminConnectClient};

    #[test]
    fn test_extract_display_name() -> Result<(), Error> {
        let resp_text = include_str!("../../tests/data/garmin_connect_display_name.html");
        let display_name = GarminConnectClient::extract_display_name(resp_text)?;
        assert_eq!(display_name.as_str(), "ddboline");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_check_versions() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let chrome_version = check_version(&config.chrome_path, "Google Chrome ").await?;
        let driver_version = check_version(&config.webdriver_path, "ChromeDriver ").await?;
        assert_eq!(chrome_version, driver_version);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_proxy_get_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let mut session = GarminConnectClient::new(config.clone());
        session.init().await?;
        let local = DateTimeWrapper::local_tz();
        let user_summary = session
            .get_user_summary(
                (OffsetDateTime::now_utc() - Duration::days(1))
                    .to_timezone(local)
                    .date(),
            )
            .await?;
        assert_eq!(user_summary.user_profile_id, 1377808);

        let max_timestamp = OffsetDateTime::now_utc() - Duration::days(14);
        let result = match session.get_activities(Some(max_timestamp)).await {
            Ok(r) => r,
            Err(_) => {
                println!("try reauth");
                session.authorize().await?;
                session.get_activities(Some(max_timestamp)).await?
            }
        };
        assert!(result.len() > 0);

        let config = GarminConfig::get_config(None)?;

        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = GarminConnectActivity::read_from_db(&pool, None, None)
            .await?
            .map_ok(|activity| (activity.activity_id, activity))
            .try_collect()
            .await?;

        let max_timestamp = OffsetDateTime::now_utc() - Duration::days(30);
        let new_activities: Vec<_> = session
            .get_activities(Some(max_timestamp))
            .await?
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.activity_id))
            .collect();
        debug!("{:?}", new_activities);
        let futures = new_activities.iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(())
            }
        });
        let results: Result<Vec<()>, Error> = try_join_all(futures).await;
        results?;
        assert_eq!(new_activities.len(), 0);

        session.close().await?;
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
