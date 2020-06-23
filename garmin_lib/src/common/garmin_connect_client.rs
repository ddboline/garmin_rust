use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use futures::future::try_join_all;
use log::debug;
use maplit::hashmap;
use postgres_query::{FromSqlRow, Parameter};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::HashMap, path::PathBuf, thread::sleep, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt};

use super::{garmin_config::GarminConfig, pgpool::PgPool, reqwest_session::ReqwestSession};

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
            let new_entries: Vec<GarminConnectActivity> = self
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

#[derive(Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(i64, Option<i32>)>>,
}

#[derive(Serialize, Deserialize, Debug, FromSqlRow)]
pub struct GarminConnectActivity {
    #[serde(rename = "activityId")]
    pub activity_id: i64,
    #[serde(rename = "activityName")]
    pub activity_name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "startTimeGMT", deserialize_with = "deserialize_start_time")]
    pub start_time_gmt: DateTime<Utc>,
    pub distance: Option<f64>,
    pub duration: f64,
    #[serde(rename = "elapsedDuration")]
    pub elapsed_duration: Option<f64>,
    #[serde(rename = "movingDuration")]
    pub moving_duration: Option<f64>,
    pub steps: Option<i64>,
    pub calories: Option<f64>,
    #[serde(rename = "averageHR")]
    pub average_hr: Option<f64>,
    #[serde(rename = "maxHR")]
    pub max_hr: Option<f64>,
}

impl GarminConnectActivity {
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM garmin_connect_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_time_gmt) >= $start_date".to_string());
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_time_gmt) <= $end_date".to_string());
            bindings.push(("end_date", d));
        }
        let query = format!(
            "{} {} ORDER BY start_time_gmt",
            query,
            if conditions.is_empty() {
                "".to_string()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            }
        );
        let query_bindings: Vec<_> = bindings.iter().map(|(k, v)| (*k, v as Parameter)).collect();
        debug!("query:\n{}", query);
        let query = postgres_query::query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .par_iter()
            .map(|r| Self::from_row(r).map_err(Into::into))
            .collect()
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
                INSERT INTO garmin_connect_activities (
                    activity_id,activity_name,description,start_time_gmt,distance,duration,
                    elapsed_duration,moving_duration,steps,calories,average_hr,max_hr
                )
                VALUES (
                    $activity_id,$activity_name,$description,$start_time_gmt,$distance,$duration,
                    $elapsed_duration,$moving_duration,$steps,$calories,$average_hr,$max_hr
                )",
            activity_id = self.activity_id,
            activity_name = self.activity_name,
            description = self.description,
            start_time_gmt = self.start_time_gmt,
            distance = self.distance,
            duration = self.duration,
            elapsed_duration = self.elapsed_duration,
            moving_duration = self.moving_duration,
            steps = self.steps,
            calories = self.calories,
            average_hr = self.average_hr,
            max_hr = self.max_hr,
        );

        let conn = pool.get().await?;

        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
                UPDATE garmin_connect_activities SET
                    activity_name=$activity_name,description=$description,
                    start_time_gmt=$start_time_gmt,distance=$distance,duration=$duration,
                    elapsed_duration=$elapsed_duration,moving_duration=$moving_duration,
                    steps=$steps,calories=$calories,average_hr=$average_hr,max_hr=$max_hr
                WHERE activity_id=$activity_id
            ",
            activity_id = self.activity_id,
            activity_name = self.activity_name,
            description = self.description,
            start_time_gmt = self.start_time_gmt,
            distance = self.distance,
            duration = self.duration,
            elapsed_duration = self.elapsed_duration,
            moving_duration = self.moving_duration,
            steps = self.steps,
            calories = self.calories,
            average_hr = self.average_hr,
            max_hr = self.max_hr,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters()).await?;
        Ok(())
    }

    pub async fn upsert_activities(
        activities: &[Self],
        pool: &PgPool,
    ) -> Result<Vec<String>, Error> {
        let mut output = Vec::new();
        let existing_activities: HashMap<_, _> = Self::read_from_db(pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.activity_id, activity))
            .collect();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.activity_id));

        let futures = update_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.update_db(&pool).await?;
                Ok(activity.activity_id.to_string())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(activity.activity_id.to_string())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        Ok(output)
    }
}

pub fn deserialize_start_time<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
        .map(|datetime| DateTime::from_utc(datetime, Utc))
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Duration, Utc};
    use futures::future::try_join_all;
    use std::collections::HashMap;

    use crate::common::{
        garmin_config::GarminConfig,
        garmin_connect_client::{GarminConnectActivity, GarminConnectClient},
        pgpool::PgPool,
    };

    #[tokio::test]
    #[ignore]
    async fn test_get_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let session = GarminConnectClient::get_session(config).await?;
        let max_timestamp = Utc::now() - Duration::days(14);
        let result = session.get_activities(max_timestamp).await?;
        assert!(result.len() > 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_dump_connect_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;

        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = GarminConnectActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.activity_id, activity))
            .collect();

        let session = GarminConnectClient::get_session(config).await?;
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
