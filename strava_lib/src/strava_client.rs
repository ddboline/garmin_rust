use anyhow::{format_err, Error};
use base64::{encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, NaiveDate, Utc};
use lazy_static::lazy_static;
use log::debug;
use maplit::hashmap;
use postgres_query::{FromSqlRow, Parameter};
use rand::{thread_rng, Rng};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use reqwest::{
    header::HeaderMap,
    multipart::{Form, Part},
    Client, Url,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};
use tempfile::Builder;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};

use garmin_lib::{
    common::{garmin_config::GarminConfig, pgpool::PgPool, strava_sync::StravaItem},
    utils::{
        garmin_util::gzip_file,
        iso_8601_datetime,
        sport_types::{deserialize_to_sport_type, SportTypes},
        stack_string::StackString,
    },
};

lazy_static! {
    static ref CSRF_TOKEN: Mutex<Option<StackString>> = Mutex::new(None);
}

pub struct LocalStravaItem(pub StravaItem);

#[derive(Debug, Copy, Clone)]
pub enum StravaAuthType {
    Read,
    Write,
}

impl Default for StravaAuthType {
    fn default() -> Self {
        Self::Read
    }
}

#[derive(Default, Debug)]
pub struct StravaClient {
    pub config: GarminConfig,
    pub client_id: StackString,
    pub client_secret: StackString,
    pub access_token: Option<StackString>,
    pub refresh_token: Option<StackString>,
    pub client: Client,
}

impl StravaClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn with_auth(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self::from_file(config).await?;
        if client.get_strava_athlete().await.is_err() {
            client.refresh_access_token().await?;
            client.to_file().await?;
        }
        Ok(client)
    }

    pub async fn from_file(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self {
            config,
            ..Self::default()
        };
        let f = File::open(&client.config.strava_tokenfile).await?;
        let mut b = BufReader::new(f);
        let mut line = String::new();
        loop {
            line.clear();
            if b.read_line(&mut line).await? == 0 {
                break;
            }
            let mut items = line.split('=');
            if let Some(key) = items.next() {
                if let Some(val) = items.next() {
                    match key.trim() {
                        "client_id" => client.client_id = val.trim().into(),
                        "client_secret" => client.client_secret = val.trim().into(),
                        "access_token" => client.access_token = Some(val.trim().into()),
                        "refresh_token" => client.refresh_token = Some(val.trim().into()),
                        _ => {}
                    }
                }
            }
        }
        Ok(client)
    }

    pub async fn to_file(&self) -> Result<(), Error> {
        let mut f = File::create(&self.config.strava_tokenfile).await?;
        f.write_all(b"[API]\n").await?;
        f.write_all(format!("client_id = {}\n", self.client_id).as_bytes())
            .await?;
        f.write_all(format!("client_secret = {}\n", self.client_secret).as_bytes())
            .await?;
        if let Some(token) = self.access_token.as_ref() {
            f.write_all(format!("access_token = {}\n", token).as_bytes())
                .await?;
        }
        if let Some(token) = self.refresh_token.as_ref() {
            f.write_all(format!("refresh_token = {}\n", token).as_bytes())
                .await?;
        }
        Ok(())
    }

    fn get_random_string() -> String {
        let random_bytes: Vec<u8> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD)
    }

    pub async fn get_authorization_url_api(&self) -> Result<Url, Error> {
        let redirect_uri = format!("https://{}/garmin/strava/callback", &self.config.domain);
        let state = Self::get_random_string();
        let url = Url::parse_with_params(
            "https://www.strava.com/oauth/authorize",
            &[
                ("client_id", self.client_id.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("response_type", "code"),
                ("approval_prompt", "auto"),
                ("scope", "activity:read_all,activity:write"),
                ("state", state.as_str()),
            ],
        )?;
        CSRF_TOKEN.lock().await.replace(state.into());
        Ok(url)
    }

    pub async fn process_callback(&mut self, code: &str, state: &str) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: StackString,
            refresh_token: StackString,
        }

        let current_state = CSRF_TOKEN.lock().await.take();
        if let Some(current_state) = current_state {
            if state != current_state.as_str() {
                return Err(format_err!("Incorrect state"));
            }
            let url = "https://www.strava.com/oauth/token";
            let data = hashmap! {
                "client_id" => self.client_id.as_str(),
                "client_secret" => self.client_secret.as_str(),
                "code" => code,
                "grant_type" => "authorization_code",
            };
            let resp: TokenResponse = self
                .client
                .post(url)
                .form(&data)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            self.access_token.replace(resp.access_token);
            self.refresh_token.replace(resp.refresh_token);
            Ok(())
        } else {
            Err(format_err!("No state"))
        }
    }

    pub async fn refresh_access_token(&mut self) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: StackString,
            refresh_token: StackString,
        }

        if let Some(refresh_token) = self.refresh_token.as_ref() {
            let url = "https://www.strava.com/oauth/token";
            let data = hashmap! {
                "client_id" => self.client_id.as_str(),
                "client_secret" => self.client_secret.as_str(),
                "refresh_token" => refresh_token.as_str(),
                "grant_type" => "refresh_token",
            };
            let resp: TokenResponse = self
                .client
                .post(url)
                .form(&data)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            self.access_token.replace(resp.access_token);
            self.refresh_token.replace(resp.refresh_token);
            Ok(())
        } else {
            Err(format_err!("No refresh token"))
        }
    }

    fn get_auth_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        let access_token = self
            .access_token
            .as_ref()
            .ok_or_else(|| format_err!("no access token"))?;
        headers.insert("Authorization", format!("Bearer {}", access_token).parse()?);
        Ok(headers)
    }

    pub async fn get_strava_athlete(&self) -> Result<StravaAthlete, Error> {
        let url = Url::parse("https://www.strava.com/api/v3/athlete")?;
        let headers = self.get_auth_headers()?;
        self.client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_strava_activities(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
        page: usize,
    ) -> Result<Vec<StravaActivity>, Error> {
        let mut params = Vec::new();
        params.push(("page", page.to_string()));
        if let Some(start_date) = start_date {
            params.push(("after", start_date.timestamp().to_string()));
        }
        if let Some(end_date) = end_date {
            params.push(("before", end_date.timestamp().to_string()));
        }

        let headers = self.get_auth_headers()?;
        let url =
            Url::parse_with_params("https://www.strava.com/api/v3/athlete/activities", &params)?;
        self.client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_all_strava_activites(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> Result<Vec<StravaActivity>, Error> {
        let mut page = 1;
        let mut activities = Vec::new();
        loop {
            let new_activities = self
                .get_strava_activities(start_date, end_date, page)
                .await?;
            if new_activities.is_empty() {
                break;
            }
            page += 1;
            activities.extend_from_slice(&new_activities);
        }
        Ok(activities)
    }

    pub async fn get_strava_activity_map(
        &self,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> Result<HashMap<StackString, StravaItem>, Error> {
        let activity_map: HashMap<_, _> = self
            .get_all_strava_activites(start_date, end_date)
            .await?
            .into_iter()
            .map(|act| {
                (
                    act.id.to_string().into(),
                    StravaItem {
                        begin_datetime: act.start_date,
                        title: act.name.into(),
                    },
                )
            })
            .collect();
        Ok(activity_map)
    }

    #[allow(clippy::similar_names)]
    pub async fn upload_strava_activity(
        &self,
        filepath: &Path,
        title: &str,
        description: &str,
    ) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct UploadResp {
            activity_id: i64,
        }

        let mut _tempfile: Option<_> = None;

        let ext = filepath
            .extension()
            .ok_or_else(|| format_err!("No extension"))?
            .to_string_lossy()
            .to_string();

        let filename = if &ext == "gz" {
            filepath.canonicalize()?.to_string_lossy().to_string()
        } else {
            let tfile = Builder::new().suffix(&format!("{}.gz", ext)).tempfile()?;
            let infname = filepath.canonicalize()?.to_string_lossy().to_string();
            let outfname = tfile.path().to_string_lossy().to_string();
            gzip_file(&infname, &outfname)?;
            _tempfile = Some(tfile);
            outfname
        };

        let fext = if filepath.ends_with("fit.gz") {
            "fit.gz"
        } else if filepath.ends_with("tcx.gz") {
            "tcx.gz"
        } else {
            return Ok("".to_string());
        };

        let part = Part::bytes(tokio::fs::read(&filename).await?).file_name(filename);
        let form = Form::new()
            .part("file", part)
            .text("name", title.to_string())
            .text("description", description.to_string())
            .text("trainer", "false")
            .text("commute", "false")
            .text("data_type", fext.to_string())
            .text("external_id", uuid::Uuid::new_v4().to_string());

        let headers = self.get_auth_headers()?;
        let url = "https://www.strava.com/api/v3/uploads";
        let resp: UploadResp = self
            .client
            .post(url)
            .multipart(form)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let url = format!(
            "https://www.strava.com/activities/{}",
            resp.activity_id.to_string()
        );

        Ok(url)
    }

    pub async fn update_strava_activity(
        &self,
        activity_id: u64,
        title: &str,
        description: Option<&str>,
        sport: SportTypes,
    ) -> Result<String, Error> {
        #[derive(Serialize)]
        struct UpdatableActivity {
            id: u64,
            commute: bool,
            trainer: bool,
            description: Option<String>,
            name: String,
            #[serde(alias = "type")]
            activity_type: String,
            gear_id: Option<String>,
        }

        let data = UpdatableActivity {
            id: activity_id,
            commute: false,
            trainer: false,
            description: description.map(ToString::to_string),
            name: title.to_string(),
            activity_type: sport.to_strava_activity(),
            gear_id: None,
        };

        let headers = self.get_auth_headers()?;
        let url = format!("https://www.strava.com/api/v3/activities/{}", activity_id);
        self.client
            .put(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        let url = format!("https://{}/garmin/strava_sync", self.config.domain);
        Ok(url)
    }
}

#[derive(Serialize, Deserialize, FromSqlRow, Debug, Clone)]
pub struct StravaActivity {
    pub name: String,
    #[serde(with = "iso_8601_datetime")]
    pub start_date: DateTime<Utc>,
    pub id: i64,
    pub distance: Option<f64>,
    pub moving_time: Option<i64>,
    pub elapsed_time: i64,
    pub total_elevation_gain: Option<f64>,
    pub elev_high: Option<f64>,
    pub elev_low: Option<f64>,
    #[serde(rename = "type", deserialize_with = "deserialize_to_sport_type")]
    pub activity_type: SportTypes,
    pub timezone: String,
}

impl StravaActivity {
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM strava_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_date) >= $start_date".to_string());
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_date) <= $end_date".to_string());
            bindings.push(("end_date", d));
        }
        let query = format!(
            "{} {} ORDER BY start_date",
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
                INSERT INTO strava_activities (
                    id,name,start_date,distance,moving_time,elapsed_time,
                    total_elevation_gain,elev_high,elev_low,activity_type,timezone
                )
                VALUES (
                    $id,$name,$start_date,$distance,$moving_time,$elapsed_time,
                    $total_elevation_gain,$elev_high,$elev_low,$activity_type,$timezone
                )",
            id = self.id,
            name = self.name,
            start_date = self.start_date,
            distance = self.distance,
            moving_time = self.moving_time,
            elapsed_time = self.elapsed_time,
            total_elevation_gain = self.total_elevation_gain,
            elev_high = self.elev_high,
            elev_low = self.elev_low,
            activity_type = self.activity_type,
            timezone = self.timezone,
        );

        let conn = pool.get().await?;

        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct StravaAthlete {
    pub id: u64,
    pub username: String,
    pub firstname: String,
    pub lastname: String,
    pub city: String,
    pub state: String,
    pub sex: String,
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{DateTime, Utc};
    use futures::future::try_join_all;
    use log::debug;
    use std::collections::HashMap;

    use garmin_lib::{
        common::{garmin_config::GarminConfig, pgpool::PgPool},
        utils::sport_types::SportTypes,
    };

    use crate::strava_client::{StravaActivity, StravaClient};

    #[tokio::test]
    #[ignore]
    async fn test_get_all_strava_activites() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let mut client = StravaClient::from_file(config).await?;
        client.refresh_access_token().await?;
        let activities = client.get_all_strava_activites(None, None).await?;
        assert!(activities.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_update_strava_activity() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::with_auth(config).await?;
        let activities = client.get_all_strava_activites(None, None).await?;
        if let Some(activity) = activities.into_iter().nth(0) {
            debug!("{} {}", activity.id, activity.name);
            let result = client
                .update_strava_activity(
                    activity.id as u64,
                    activity.name.as_str(),
                    Some("Test description"),
                    SportTypes::Running,
                )
                .await?;
            debug!("{}", result);
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_strava_athlete() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::with_auth(config).await?;
        let athlete = client.get_strava_athlete().await?;
        assert_eq!(athlete.username.as_str(), "dboline");
        assert_eq!(athlete.id, 3532812);
        assert_eq!(athlete.firstname.as_str(), "Daniel");
        assert_eq!(athlete.lastname.as_str(), "Boline");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_dump_strava_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = StravaActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.id, activity))
            .collect();
        let client = StravaClient::with_auth(config).await?;
        let start_date: DateTime<Utc> = "2017-01-01T00:00:00Z".parse()?;
        let new_activities: Vec<_> = client
            .get_all_strava_activites(Some(start_date), None)
            .await?
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.id))
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
