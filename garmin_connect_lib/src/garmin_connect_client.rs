use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, Utc, Duration};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use log::{debug, error};
use maplit::hashmap;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
    Response,
};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt, sync::Mutex};

use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
};

use super::reqwest_session::ReqwestSession;

const BASE_URL: &str = "https://connect.garmin.com";
const SSO_URL: &str = "https://sso.garmin.com/sso";
const MODERN_URL: &str = "https://connect.garmin.com/modern";
const SIGNIN_URL: &str = "https://sso.garmin.com/sso/signin";

const CONNECT_SESSION_TIMEOUT: i64 = 3600;

lazy_static! {
    static ref CONNECT_SESSION: Mutex<GarminConnectClient> =
        Mutex::new(GarminConnectClient::default());
}

#[derive(Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(i64, Option<i32>)>>,
}

#[derive(Clone)]
pub struct GarminConnectClient {
    pub config: GarminConfig,
    session: ReqwestSession,
    display_name: Option<StackString>,
    auth_time: Option<DateTime<Utc>>,
    retry_time: DateTime<Utc>,
    auth_trigger: Arc<AtomicBool>,
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
            session: ReqwestSession::new(true),
            display_name: None,
            auth_time: None,
            retry_time: Utc::now(),
            auth_trigger: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn authorize(&mut self) -> Result<(), Error> {
        if Utc::now() < self.retry_time {
            return Err(format_err!("Please retry after {}", self.retry_time));
        }

        let params = hashmap! {
            "webhost" => BASE_URL,
            "service" => MODERN_URL,
            "source" => SIGNIN_URL,
            "redirectAfterAccountLoginUrl" => MODERN_URL,
            "redirectAfterAccountCreationUrl" => MODERN_URL,
            "gauthHost" => SSO_URL,
            "locale" => "en_US",
            "id" => "gauth-widget",
            "cssUrl" => "https://static.garmincdn.com/com.garmin.connect/ui/css/gauth-custom-v1.2-min.css",
            "clientId" => "GarminConnect",
            "rememberMeShown" => "true",
            "rememberMeChecked" => "false",
            "createAccountShown" => "true",
            "openCreateAccount" => "false",
            "usernameShown" => "false",
            "displayNameShown" => "false",
            "consumeServiceTicket" => "false",
            "initialFocus" => "true",
            "embedWidget" => "false",
            "generateExtraServiceTicket" => "false",
        };

        let data = hashmap! {
            "username" => self.config.garmin_connect_email.as_str(),
            "password" => self.config.garmin_connect_password.as_str(),
            "embed" => "true",
            "lt" => "e1s1",
            "_eventId" => "submit",
            "displayNameRequired" => "false",
        };

        let garmin_signin_headers = hashmap! {
            "User-Agent" => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/79.0.3945.88 Safari/537.36",
            "origin" => "https://sso.garmin.com",
        };

        let url = Url::parse_with_params(SIGNIN_URL, params.iter())?;
        let signin_headers: Result<HeaderMap<_>, Error> = garmin_signin_headers
            .into_iter()
            .map(|(k, v)| {
                let name: HeaderName = k.parse()?;
                let val: HeaderValue = v.parse()?;
                Ok((name, val))
            })
            .collect();
        let signin_headers = signin_headers?;

        let resp = self
            .session
            .post_no_retry(&url, &signin_headers, &data)
            .await?;
        let status = resp.status();
        if status == 429 {
            self.retry_time = Self::get_next_attempt_time(resp.headers())?;
            return Err(format_err!("Too many requests, try again after {}", self.retry_time));
        }

        let resp_text = resp.error_for_status()?.text().await?;
        Self::check_signin_response(status.into(), &resp_text)?;
        let response_url = Self::get_response_url(&resp_text)?;

        let resp = self.session.get_no_retry(
            &response_url,
            &HeaderMap::new(),
        ).await?;

        if resp.status() == 429 {
            self.retry_time = Self::get_next_attempt_time(resp.headers())?;
            return Err(format_err!("Too many requests, try again after {}", self.retry_time));
        }

        let resp_text = resp.error_for_status()?.text().await?;

        self.display_name.replace(Self::extract_display_name(&resp_text)?);

        self.auth_time = Some(Utc::now());
        self.auth_trigger.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn get_next_attempt_time(headers: &HeaderMap) -> Result<DateTime<Utc>, Error> {
        let retry_after = if let Some(retry_after) = headers.get("retry-after") {
            retry_after.to_str()?.parse()?
        } else {
            60
        };
        Ok(Utc::now() + Duration::seconds(retry_after))
    }

    fn check_signin_response(status: u16, text: &str) -> Result<(), Error> {
        if text.contains("temporarily unavailable") {
            return Err(format_err!("SSO error {} {}", status, text));
        } else if text.contains(">sendEvent('FAIL')") {
            return Err(format_err!("Invalid login"));
        } else if text.contains(">sendEvent('ACCOUNT_LOCKED')") {
            return Err(format_err!("Account Locked"));
        } else if text.contains("renewPassword") {
            return Err(format_err!("Reset password"));
        }
        Ok(())
    }

    fn get_response_url(text: &str) -> Result<Url, Error> {
        for line in text.split('\n') {
            if line.contains("var response_url") {
                let new_line = line
                    .replace(r#"""#, "")
                    .replace(r#"\/"#, "/")
                    .replace(";", "");
                let url = new_line.split_whitespace().last().unwrap_or_else(|| "");
                return Url::parse(url).map_err(Into::into);
            }
        }

        Err(format_err!("NO URL FOUND"))
    }

    fn extract_display_name(text: &str) -> Result<StackString, Error> {
        for entry in text.split('\n').filter(|x| x.contains("JSON.parse")) {
            let entry = entry.replace(r#"\""#, r#"""#).replace(r#"");"#, "");
            let entries: Vec<_> = entry.split(r#" = JSON.parse(""#).take(2).collect();
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
        &self,
        date: NaiveDate,
    ) -> Result<GarminConnectUserDailySummary, Error> {
        let display_name = self
            .display_name
            .as_ref()
            .ok_or_else(|| format_err!("No display name"))?;
        let url_prefix = format!(
            "{}/proxy/usersummary-service/usersummary/daily/{}",
            MODERN_URL, display_name,
        );
        let url = Url::parse_with_params(&url_prefix, &[("calendarDate", &date.to_string())])?;
        let resp = self.session.get_no_retry(&url, &HeaderMap::new()).await?;
        if resp.status() == 403 {
            self.auth_trigger.store(true, Ordering::SeqCst);
            error!("trigger re-auth");
        }
        let user_summary = resp.error_for_status()?.json().await?;
        Ok(user_summary)
    }

    pub async fn get_heartrate(&self, date: NaiveDate) -> Result<GarminConnectHrData, Error> {
        let display_name = self
            .display_name
            .as_ref()
            .ok_or_else(|| format_err!("No display name"))?;
        let url_prefix = format!(
            "{}/proxy/wellness-service/wellness/dailyHeartRate/{}",
            MODERN_URL, display_name
        );
        let url = Url::parse_with_params(&url_prefix, &[("date", &date.to_string())])?;
        let resp = self.session.get(&url, &HeaderMap::new()).await?;
        if resp.status() == 403 {
            self.auth_trigger.store(true, Ordering::SeqCst);
            error!("trigger re-auth");
        }
        resp.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn get_activities(
        &self,
        max_timestamp: DateTime<Utc>,
    ) -> Result<Vec<GarminConnectActivity>, Error> {
        let url_prefix = format!(
            "{}/proxy/activitylist-service/activities/search/activities",
            MODERN_URL
        );
        let mut entries = Vec::new();
        let mut current_start = 0;
        let limit = 10;
        loop {
            let url = Url::parse_with_params(
                &url_prefix,
                &[
                    ("start", current_start.to_string()),
                    ("limit", limit.to_string()),
                ],
            )?;
            current_start += limit;
            debug!("Call {}", url);
            let resp = self.session.get(&url, &HeaderMap::new()).await?;
            if resp.status() == 403 {
                println!("{:?}", resp);
                println!("{}", resp.text().await?);
                self.auth_trigger.store(true, Ordering::SeqCst);
                error!("trigger re-auth");
                return Err(format_err!("No auth for some reason"));
            }

            let new_entries: Vec<GarminConnectActivity> = resp.error_for_status()?.json().await?;
            if new_entries.is_empty() {
                debug!("Empty result {} returning {} results", url, entries.len());
                return Ok(entries);
            }
            for entry in new_entries {
                if entry.start_time_gmt > max_timestamp {
                    debug!("{} {}", entry.activity_id, entry.start_time_gmt);
                    entries.push(entry);
                } else {
                    debug!("Returning {} results", entries.len());
                    return Ok(entries);
                }
            }
        }
    }

    pub async fn get_activity_files(
        &self,
        activities: &[GarminConnectActivity],
    ) -> Result<Vec<PathBuf>, Error> {
        let futures = activities.iter().map(|activity| async move {
            let fname = self
                .config
                .home_dir
                .join("Downloads")
                .join(activity.activity_id.to_string())
                .with_extension("zip");
            let url: Url = format!(
                "{}/{}/{}",
                BASE_URL, "modern/proxy/download-service/files/activity", activity.activity_id
            )
            .parse()?;
            let mut f = File::create(&fname).await?;
            let resp = self
                .session
                .get(&url, &HeaderMap::new())
                .await?
                .error_for_status()?;

            let mut stream = resp.bytes_stream();
            while let Some(item) = stream.next().await {
                f.write_all(&item?).await?;
            }
            Ok(fname)
        });
        try_join_all(futures).await
    }
}

pub async fn get_garmin_connect_session(
    config: &GarminConfig,
) -> Result<GarminConnectClient, Error> {
    fn has_timed_out(t: DateTime<Utc>) -> bool {
        (Utc::now() - t).num_seconds() > CONNECT_SESSION_TIMEOUT
    }

    let mut session = CONNECT_SESSION.lock().await.clone();
    session.config = config.clone();

    let auth_trigger = session.auth_trigger.load(Ordering::SeqCst);

    if auth_trigger {
        error!("re-auth session");
    }

    // if session is old, OR hasn't been authorized, OR get_user_summary fails, then
    // reauthorize
    if auth_trigger
        || session.auth_time.map_or(true, has_timed_out)
        || session
            .get_user_summary(Utc::now().naive_local().date())
            .await
            .is_err()
    {
        let mut session_guard = CONNECT_SESSION.lock().await;
        session_guard.config = config.clone();
        session_guard.authorize().await?;
        session = session_guard.clone();
    }

    Ok(session)
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

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::garmin_connect_client::{
        get_garmin_connect_session, GarminConnectActivity, GarminConnectClient,
    };

    #[tokio::test]
    #[ignore]
    async fn test_get_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let session = get_garmin_connect_session(&config).await?;
        let max_timestamp = Utc::now() - Duration::days(14);
        let result = session.get_activities(max_timestamp).await?;
        assert!(result.len() > 0);

        let config = GarminConfig::get_config(None)?;
        let session = get_garmin_connect_session(&config).await?;
        let user_summary = session
            .get_user_summary((Utc::now() - Duration::days(1)).naive_local().date())
            .await?;
        assert_eq!(user_summary.user_profile_id, 1377808);

        let config = GarminConfig::get_config(None)?;

        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = GarminConnectActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.activity_id, activity))
            .collect();

        let session = get_garmin_connect_session(&config).await?;
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

    #[test]
    fn test_get_response_url() -> Result<(), Error> {
        let resp_text = include_str!("../../tests/data/garmin_auth_response.html");
        let url = GarminConnectClient::get_response_url(resp_text)?;
        assert_eq!(
            url.as_str(),
            "https://connect.garmin.com/modern?ticket=ST-0765302-muiAj3bYsqmU1TyBFMZB-cas"
        );
        Ok(())
    }

    #[test]
    fn test_extract_display_name() -> Result<(), Error> {
        let resp_text = include_str!("../../tests/data/garmin_connect_display_name.html");
        let display_name = GarminConnectClient::extract_display_name(resp_text)?;
        assert_eq!(display_name.as_str(), "ddboline");
        Ok(())
    }
}
