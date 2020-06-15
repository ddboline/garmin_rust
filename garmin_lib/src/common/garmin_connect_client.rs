use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use log::debug;
use maplit::hashmap;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, thread::sleep, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt};

use super::{garmin_config::GarminConfig, reqwest_session::ReqwestSession};

const GARMIN_PREFIX: &str = "https://connect.garmin.com/modern";

#[derive(Clone)]
pub struct GarminConnectClient {
    pub config: GarminConfig,
    session: ReqwestSession,
    display_name: Option<String>,
}

impl GarminConnectClient {
    pub async fn get_session(config: GarminConfig) -> Result<Self, Error> {
        let obligatory_headers = hashmap! {
            "Referer" => "https://sync.tapiriik.com",
        };
        let garmin_signin_headers = hashmap! {
            "origin" => "https://sso.garmin.com",
        };

        let data = hashmap! {
            "username" => config.garmin_connect_email.as_str(),
            "password" => config.garmin_connect_password.as_str(),
            "_eventId" => "submit",
            "embed" => "true",
        };

        let params = hashmap! {
            "service"=> GARMIN_PREFIX,
            "clientId"=> "GarminConnect",
            "gauthHost"=>"https://sso.garmin.com/sso",
            "consumeServiceTicket"=>"false",
        };

        let session = ReqwestSession::new(false);

        let url = Url::parse_with_params("https://sso.garmin.com/sso/signin", params.iter())?;
        let pre_resp = session.get(&url, &HeaderMap::new()).await?;
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

        let sso_resp = session.post(&url, &signin_headers, &data).await?;
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

        let mut gc_redeem_resp = session
            .get(&GARMIN_PREFIX.parse()?, &HeaderMap::new())
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
        let mut display_name = None;
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
            gc_redeem_resp = session.get(&url, &HeaderMap::new()).await?;
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
                            display_name: String,
                        }
                        let val: SocialProfile = serde_json::from_str(entries[1])?;
                        display_name.replace(val.display_name);
                    }
                }
                break;
            }
            current_redirect_count += 1;
            if current_redirect_count > max_redirect_count {
                break;
            }
        }

        session.set_default_headers(obligatory_headers).await?;

        Ok(Self {
            config,
            session,
            display_name,
        })
    }

    pub async fn get_user_summary(&self, date: NaiveDate) -> Result<(), Error> {
        let display_name = self
            .display_name
            .as_ref()
            .ok_or_else(|| format_err!("No display name"))?;
        let url_prefix = format!(
            "{}/proxy/usersummary-service/usersummary/daily/{}",
            GARMIN_PREFIX, display_name,
        );
        let url = Url::parse_with_params(&url_prefix, &[("calendarDate", &date.to_string())])?;
        self.session
            .get(&url, &HeaderMap::new())
            .await?
            .error_for_status()?;
        Ok(())
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
        self.session
            .get(&url, &HeaderMap::new())
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_activities(&self, max_timestamp: DateTime<Utc>) -> Result<Vec<String>, Error> {
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
            let new_entries: Vec<HashMap<String, Value>> = self
                .session
                .get(&url, &HeaderMap::new())
                .await?
                .error_for_status()?
                .json()
                .await?;
            if new_entries.is_empty() {
                debug!("Empty result {} returning {} results", url, entries.len());
                return Ok(entries);
            }
            for entry in &new_entries {
                if let Some(activity_id) = entry.get("activityId") {
                    if let Some(start_time_gmt) = entry.get("startTimeGMT").and_then(Value::as_str)
                    {
                        let start_time: DateTime<Utc> =
                            NaiveDateTime::parse_from_str(start_time_gmt, "%Y-%m-%d %H:%M:%S")
                                .map(|datetime| DateTime::from_utc(datetime, Utc))?;
                        if start_time > max_timestamp {
                            debug!("{} {}", activity_id, start_time);
                            let fname = self
                                .config
                                .home_dir
                                .join("Downloads")
                                .join(format!("{}.zip", activity_id));
                            let url: Url = format!(
                                "{}/{}/{}",
                                "https://connect.garmin.com",
                                "modern/proxy/download-service/files/activity",
                                activity_id
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

                            entries.push(fname.to_string_lossy().to_string());
                        } else {
                            debug!("Returning {} results", entries.len());
                            return Ok(entries);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(i64, Option<i32>)>>,
}
