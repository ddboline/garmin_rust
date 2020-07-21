use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use log::{debug, error};
use maplit::hashmap;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::sleep,
    time::Duration,
};
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt, sync::Mutex};

use garmin_lib::common::{
    garmin_config::GarminConfig, garmin_connect_activity::GarminConnectActivity,
};

use super::reqwest_session::ReqwestSession;

const GARMIN_PREFIX: &str = "https://connect.garmin.com/modern";
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
            session: ReqwestSession::new(false),
            display_name: None,
            auth_time: None,
            auth_trigger: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn authorize(&mut self) -> Result<(), Error> {
        let obligatory_headers = hashmap! {
            "Referer" => "https://sync.tapiriik.com",
        };
        let garmin_signin_headers = hashmap! {
            "origin" => "https://sso.garmin.com",
        };

        let data = hashmap! {
            "username" => self.config.garmin_connect_email.as_str(),
            "password" => self.config.garmin_connect_password.as_str(),
            "_eventId" => "submit",
            "embed" => "true",
        };

        let params = hashmap! {
            "service"=> GARMIN_PREFIX,
            "clientId"=> "GarminConnect",
            "gauthHost"=>"https://sso.garmin.com/sso",
            "consumeServiceTicket"=>"false",
        };

        let url = Url::parse_with_params("https://sso.garmin.com/sso/signin", params.iter())?;
        let pre_resp = self.session.get_no_retry(&url, &HeaderMap::new()).await?;
        if pre_resp.status() != 200 {
            return Err(format_err!(
                "SSO prestart error {} {}",
                pre_resp.status(),
                pre_resp.text().await?
            ));
        }

        let result: Result<HeaderMap<_>, Error> = garmin_signin_headers
            .into_iter()
            .map(|(k, v)| {
                let name: HeaderName = k.parse()?;
                let val: HeaderValue = v.parse()?;
                Ok((name, val))
            })
            .collect();
        let signin_headers = result?;

        let sso_resp = self
            .session
            .post_no_retry(&url, &signin_headers, &data)
            .await?;
        let status = sso_resp.status();
        if status != 200 {
            return Err(format_err!(
                "SSO error {} {:?} {}",
                status,
                sso_resp.headers().clone(),
                sso_resp.text().await?
            ));
        }

        let sso_text = sso_resp.text().await?;

        if sso_text.contains("temporarily unavailable") {
            return Err(format_err!("SSO error {} {}", status, sso_text));
        } else if sso_text.contains(">sendEvent('FAIL')") {
            return Err(format_err!("Invalid login"));
        } else if sso_text.contains(">sendEvent('ACCOUNT_LOCKED')") {
            return Err(format_err!("Account Locked"));
        } else if sso_text.contains("renewPassword") {
            return Err(format_err!("Reset password"));
        }

        let mut gc_redeem_resp = self
            .session
            .get_no_retry(&GARMIN_PREFIX.parse()?, &HeaderMap::new())
            .await?;
        if gc_redeem_resp.status() != 302 {
            return Err(format_err!(
                "GC redeem-start error {} {}",
                gc_redeem_resp.status(),
                gc_redeem_resp.text().await?
            ));
        }

        let mut url_prefix = "https://connect.garmin.com".to_string();

        let max_redirect_count = 7;
        let mut current_redirect_count = 1;
        self.display_name.take();
        loop {
            sleep(Duration::from_secs(2));
            let url = gc_redeem_resp
                .headers()
                .get("location")
                .expect("No location")
                .to_str()?;
            let url = if url.starts_with('/') {
                format!("{}{}", url_prefix, url)
            } else {
                url.to_string()
            };
            url_prefix = url.split('/').take(3).collect::<Vec<_>>().join("/");

            let url: Url = url.parse()?;
            gc_redeem_resp = self.session.get_no_retry(&url, &HeaderMap::new()).await?;
            let status = gc_redeem_resp.status();
            if current_redirect_count >= max_redirect_count && status != 200 {
                return Err(format_err!(
                    "GC redeem {}/{} err {} {}",
                    current_redirect_count,
                    max_redirect_count,
                    status,
                    gc_redeem_resp.text().await?
                ));
            } else if status == 200 || status == 404 {
                let resp = gc_redeem_resp.text().await?;
                for entry in resp.split('\n').filter(|x| x.contains("JSON.parse")) {
                    let entry = entry.replace(r#"\""#, r#"""#).replace(r#"");"#, "");
                    let entries: Vec<_> = entry.split(r#" = JSON.parse(""#).take(2).collect();
                    if entries[0].contains("VIEWER_SOCIAL_PROFILE") {
                        #[derive(Deserialize)]
                        struct SocialProfile {
                            #[serde(rename = "displayName")]
                            display_name: StackString,
                        }
                        let val: SocialProfile = serde_json::from_str(entries[1])?;
                        self.display_name.replace(val.display_name);
                    }
                }
                break;
            }
            current_redirect_count += 1;
            if current_redirect_count > max_redirect_count {
                break;
            }
        }

        self.session.set_default_headers(obligatory_headers).await?;
        self.auth_time = Some(Utc::now());
        Ok(())
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
            GARMIN_PREFIX, display_name,
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
            GARMIN_PREFIX, display_name
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
            GARMIN_PREFIX
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
                self.auth_trigger.store(true, Ordering::SeqCst);
                error!("trigger re-auth");
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
                "https://connect.garmin.com",
                "modern/proxy/download-service/files/activity",
                activity.activity_id
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

    let auth_trigger = session
        .auth_trigger
        .compare_and_swap(true, false, Ordering::SeqCst);

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
        if session_guard
            .get_user_summary(Utc::now().naive_local().date())
            .await
            .is_err()
        {
            session_guard.authorize().await?;
        }
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

    use crate::garmin_connect_client::{get_garmin_connect_session, GarminConnectActivity};

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
}
