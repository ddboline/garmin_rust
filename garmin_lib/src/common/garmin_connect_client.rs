use chrono::{DateTime, NaiveDateTime, Utc};
use failure::{err_msg, format_err, Error};
use log::debug;
use maplit::hashmap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::thread::sleep;
use std::time::Duration;

use super::garmin_config::GarminConfig;
use super::reqwest_session::ReqwestSession;

pub struct GarminConnectClient {
    config: GarminConfig,
    session: ReqwestSession,
}

impl GarminConnectClient {
    pub fn get_session(config: GarminConfig) -> Result<Self, Error> {
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
            "service"=> "https://connect.garmin.com/modern",
            "clientId"=> "GarminConnect",
            "gauthHost"=>"https://sso.garmin.com/sso",
            "consumeServiceTicket"=>"false",
        };

        let session = ReqwestSession::new(false);

        let url = Url::parse_with_params("https://sso.garmin.com/sso/signin", params.iter())?;
        let pre_resp = session.get(&url, HeaderMap::new())?;
        if pre_resp.status() != 200 {
            return Err(format_err!(
                "SSO prestart error {} {}",
                pre_resp.status(),
                pre_resp.text()?
            ));
        }

        let mut signin_headers = HeaderMap::new();
        for (k, v) in garmin_signin_headers.into_iter() {
            let name: HeaderName = k.parse()?;
            let val: HeaderValue = v.parse()?;
            signin_headers.insert(name, val);
        }

        let sso_resp = session.post(&url, signin_headers, &data)?;
        let status = sso_resp.status();
        if status != 200 {
            return Err(format_err!("SSO error {} {}", status, sso_resp.text()?));
        }

        let sso_text = sso_resp.text()?;

        if sso_text.contains("temporarily unavailable") {
            return Err(format_err!("SSO error {} {}", status, sso_text));
        } else if sso_text.contains(">sendEvent('FAIL')") {
            return Err(err_msg("Invalid login"));
        } else if sso_text.contains(">sendEvent('ACCOUNT_LOCKED')") {
            return Err(err_msg("Account Locked"));
        } else if sso_text.contains("renewPassword") {
            return Err(err_msg("Reset password"));
        }

        let mut gc_redeem_resp = session.get(
            &"https://connect.garmin.com/modern".parse()?,
            HeaderMap::new(),
        )?;
        if gc_redeem_resp.status() != 302 {
            return Err(format_err!(
                "GC redeem-start error {} {}",
                gc_redeem_resp.status(),
                gc_redeem_resp.text()?
            ));
        }

        let mut url_prefix = "https://connect.garmin.com".to_string();

        let max_redirect_count = 7;
        let mut current_redirect_count = 1;
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
            gc_redeem_resp = session.get(&url, HeaderMap::new())?;
            let status = gc_redeem_resp.status();
            if current_redirect_count >= max_redirect_count && status != 200 {
                return Err(format_err!(
                    "GC redeem {}/{} err {} {}",
                    current_redirect_count,
                    max_redirect_count,
                    status,
                    gc_redeem_resp.text()?
                ));
            } else if status == 200 || status == 404 {
                break;
            }
            current_redirect_count += 1;
            if current_redirect_count > max_redirect_count {
                break;
            }
        }

        session.set_default_headers(obligatory_headers)?;

        Ok(Self { config, session })
    }

    pub fn get_activities(&self, max_timestamp: DateTime<Utc>) -> Result<Vec<String>, Error> {
        let url_prefix = "https://connect.garmin.com/modern/proxy/activitylist-service/activities/search/activities";
        let mut entries = Vec::new();
        let mut current_start = 0;
        let limit = 10;
        loop {
            let url = Url::parse_with_params(
                url_prefix,
                &[
                    ("start", current_start.to_string()),
                    ("limit", limit.to_string()),
                ],
            )?;
            current_start += limit;
            debug!("Call {}", url);
            let new_entries: Vec<HashMap<String, Value>> =
                self.session.get(&url, HeaderMap::new())?.json()?;
            if new_entries.is_empty() {
                debug!("Empty result {} returning {} results", url, entries.len());
                return Ok(entries);
            }
            for entry in &new_entries {
                if let Some(activity_id) = entry.get("activityId") {
                    if let Some(start_time_gmt) = entry.get("startTimeGMT").and_then(|x| x.as_str())
                    {
                        let start_time: DateTime<Utc> =
                            NaiveDateTime::parse_from_str(start_time_gmt, "%Y-%m-%d %H:%M:%S")
                                .map(|datetime| DateTime::from_utc(datetime, Utc))?;
                        if start_time > max_timestamp {
                            println!("{} {}", activity_id, start_time);
                            let fname =
                                format!("{}/Downloads/{}.zip", self.config.home_dir, activity_id);
                            let url: Url = format!(
                                "{}/{}/{}",
                                "https://connect.garmin.com",
                                "modern/proxy/download-service/files/activity",
                                activity_id
                            )
                            .parse()?;
                            let mut f = File::create(&fname)?;
                            let mut resp = self.session.get(&url, HeaderMap::new())?;
                            resp.copy_to(&mut f)?;
                            entries.push(fname);
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
