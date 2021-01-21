use anyhow::{format_err, Error};
use base64::{encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, Local, Utc};
use crossbeam_utils::atomic::AtomicCell;
use futures::future::try_join_all;
use lazy_static::lazy_static;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{
    header::HeaderMap,
    multipart::{Form, Part},
    Client, Url,
};
use select::{document::Document, predicate::Attr};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::StackString;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tempfile::Builder;
use tokio::{
    fs::{create_dir_all, File},
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    task::spawn_blocking,
    time::sleep,
};
use tokio_stream::StreamExt;

use garmin_lib::{
    common::{
        garmin_config::GarminConfig, garmin_summary::get_list_of_activities_from_db,
        pgpool::PgPool, strava_activity::StravaActivity,
    },
    utils::{
        garmin_util::gzip_file,
        iso_8601_datetime::convert_datetime_to_str,
        sport_types::{self, SportTypes},
    },
};

lazy_static! {
    static ref CSRF_TOKEN: AtomicCell<Option<StackString>> = AtomicCell::new(None);
    static ref WEB_CSRF: AtomicCell<Option<WebCsrf>> = AtomicCell::new(None);
}

const BASE_URL: &str = "https://www.strava.com";

#[derive(Clone, Debug)]
struct WebCsrf {
    param: StackString,
    token: StackString,
}

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
            client: Client::builder().cookie_store(true).build()?,
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
            let items: SmallVec<[&str; 2]> = line.split('=').take(2).collect();
            if let Some(key) = items.get(0) {
                if let Some(val) = items.get(1) {
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

    pub async fn webauth(&self) -> Result<(), Error> {
        let login_url: Url = format!("{}/login", BASE_URL).parse()?;
        let session_url: Url = format!("{}/session", BASE_URL).parse()?;
        let email = self
            .config
            .strava_email
            .as_ref()
            .ok_or_else(|| format_err!("No Strava Email"))?;
        let password = self
            .config
            .strava_password
            .as_ref()
            .ok_or_else(|| format_err!("No Strava Password"))?;

        let text = self
            .client
            .get(login_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let (param, token) = Self::extract_web_csrf(&text)?;
        let data = hashmap! {
            "email" => email.as_str(),
            "password" => password.as_str(),
            "remember_me" => "on",
            param.as_str() => token.as_str(),
        };
        self.client
            .post(session_url)
            .form(&data)
            .send()
            .await?
            .error_for_status()?;
        WEB_CSRF.store(Some(WebCsrf { param, token }));
        Ok(())
    }

    async fn export_original(&self, activity_id: u64) -> Result<PathBuf, Error> {
        if let Some(web_csrf) = WEB_CSRF.swap(None) {
            WEB_CSRF.swap(Some(web_csrf));
        } else {
            self.webauth().await?;
        }
        let url: Url = format!(
            "https://www.strava.com/activities/{}/export_original",
            activity_id
        )
        .parse()?;
        let resp = self.client.get(url).send().await?.error_for_status()?;

        create_dir_all(&self.config.download_directory).await?;

        let fname = self
            .config
            .download_directory
            .join(activity_id.to_string())
            .with_extension("fit");

        let mut f = File::create(&fname).await?;
        let mut stream = resp.bytes_stream();
        while let Some(item) = stream.next().await {
            f.write_all(&item?).await?;
        }

        Ok(fname)
    }

    pub async fn delete_activity(&self, activity_id: u64) -> Result<(), Error> {
        let web_csrf = if let Some(web_csrf) = WEB_CSRF.swap(None) {
            web_csrf
        } else {
            self.webauth().await?;
            if let Some(web_csrf) = WEB_CSRF.swap(None) {
                web_csrf
            } else {
                return Err(format_err!("Auth failure"));
            }
        };
        let url: Url = format!("{}/activities/{}", BASE_URL, activity_id).parse()?;
        let data = hashmap! {
            "_method" => "delete",
            web_csrf.param.as_str() => web_csrf.token.as_str(),
        };
        self.client
            .post(url)
            .form(&data)
            .send()
            .await?
            .error_for_status()?;
        WEB_CSRF.swap(Some(web_csrf));
        Ok(())
    }

    fn extract_web_csrf(text: &str) -> Result<(StackString, StackString), Error> {
        let document = Document::from(text);
        if let Some(param) = document
            .find(Attr("name", "csrf-param"))
            .next()
            .and_then(|node| node.attr("content"))
        {
            if let Some(token) = document
                .find(Attr("name", "csrf-token"))
                .next()
                .and_then(|node| node.attr("content"))
            {
                return Ok((param.into(), token.into()));
            }
        }
        Err(format_err!("No csrf token"))
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

    fn get_random_string() -> StackString {
        let random_bytes: SmallVec<[u8; 16]> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD).into()
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
        CSRF_TOKEN.store(Some(state));
        Ok(url)
    }

    pub async fn process_callback(&mut self, code: &str, state: &str) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: StackString,
            refresh_token: StackString,
        }

        if let Some(current_state) = CSRF_TOKEN.swap(None) {
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

    pub async fn create_strava_activity(&self, activity: &StravaActivity) -> Result<i64, Error> {
        #[derive(Serialize, Deserialize)]
        struct CreateActivityForm {
            name: StackString,
            #[serde(rename = "type", with = "sport_types")]
            activity_type: SportTypes,
            start_date_local: StackString,
            elapsed_time: i64,
            description: StackString,
            distance: i64,
            trainer: bool,
            commute: bool,
        }

        #[derive(Serialize, Deserialize)]
        struct CreateActivityResp {
            id: i64,
        }

        let data = CreateActivityForm {
            name: activity.name.clone(),
            activity_type: activity.activity_type,
            start_date_local: activity
                .start_date
                .with_timezone(&Local)
                .format("%Y-%m-%dT%H:%M:%S%z")
                .to_string()
                .into(),
            elapsed_time: activity.elapsed_time,
            description: "".into(),
            distance: activity.distance.map_or(0, |d| d as i64),
            trainer: false,
            commute: false,
        };

        let headers = self.get_auth_headers()?;
        let url = "https://www.strava.com/api/v3/activities";
        let resp: CreateActivityResp = self
            .client
            .post(url)
            .headers(headers)
            .form(&data)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.id)
    }

    #[allow(clippy::similar_names)]
    pub async fn upload_strava_activity(
        &self,
        filepath: &Path,
        title: &str,
        description: &str,
    ) -> Result<StackString, Error> {
        #[derive(Deserialize, Debug)]
        struct UploadResponse {
            id: u64,
            id_str: StackString,
            external_id: Option<StackString>,
            error: Option<StackString>,
            status: StackString,
            activity_id: Option<u64>,
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
            let tfile = Builder::new().suffix(&format!(".{}.gz", ext)).tempfile()?;
            let infname = filepath.canonicalize()?;
            let outfpath = tfile.path().to_path_buf();
            let outfname = outfpath.to_string_lossy().to_string();
            spawn_blocking(move || gzip_file(&infname, &outfpath)).await??;
            _tempfile = Some(tfile);
            outfname
        };

        let fext = if filename.ends_with("fit.gz") {
            "fit.gz"
        } else if filename.ends_with("tcx.gz") {
            "tcx.gz"
        } else {
            return Err(format_err!("Bad extension {}", filename));
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

        let result: UploadResponse = self
            .client
            .post(url)
            .multipart(form)
            .headers(headers.clone())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let url = format!("{}/{}", url, result.id);
        for _ in 0..10 {
            let result: UploadResponse = self
                .client
                .get(&url)
                .headers(headers.clone())
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if result.activity_id.is_some() {
                break;
            }
            sleep(std::time::Duration::from_secs(2)).await;
        }

        let url = format!("https://{}/garmin/strava_sync", self.config.domain).into();
        Ok(url)
    }

    pub async fn update_strava_activity(
        &self,
        activity_id: u64,
        title: &str,
        description: Option<&str>,
        sport: SportTypes,
        start_time: Option<DateTime<Utc>>,
    ) -> Result<StackString, Error> {
        #[derive(Serialize)]
        struct UpdatableActivity {
            id: u64,
            commute: bool,
            trainer: bool,
            description: Option<StackString>,
            name: StackString,
            #[serde(alias = "type")]
            activity_type: StackString,
            gear_id: Option<StackString>,
        }

        let data = UpdatableActivity {
            id: activity_id,
            commute: false,
            trainer: false,
            description: description.map(Into::into),
            name: title.into(),
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
        let url = if let Some(start_time) = start_time {
            let start_time = convert_datetime_to_str(start_time);
            Url::parse_with_params(
                &url,
                &[
                    ("start_datetime", &start_time),
                    ("end_datetime", &start_time),
                ],
            )?
        } else {
            url.parse()?
        };
        Ok(url.into_string().into())
    }

    pub async fn sync_with_client(
        &self,
        start_datetime: Option<DateTime<Utc>>,
        end_datetime: Option<DateTime<Utc>>,
        pool: &PgPool,
    ) -> Result<Vec<PathBuf>, Error> {
        let new_activities: Vec<_> = self
            .get_all_strava_activites(start_datetime, end_datetime)
            .await?;

        StravaActivity::upsert_activities(&new_activities, &pool).await?;
        StravaActivity::fix_summary_id_in_db(&pool).await?;

        let mut constraints: SmallVec<[String; 2]> = SmallVec::new();
        if let Some(start_datetime) = start_datetime {
            constraints.push(format!("begin_datetime >= '{}'", start_datetime));
        }
        if let Some(end_datetime) = end_datetime {
            constraints.push(format!("begin_datetime <= '{}'", end_datetime));
        }
        let constraints = constraints.join(" AND ");

        let old_activities: HashSet<_> = get_list_of_activities_from_db(&constraints, &pool)
            .await?
            .into_iter()
            .map(|(d, _)| d)
            .collect();

        #[allow(clippy::filter_map)]
        let futures = new_activities
            .into_iter()
            .filter_map(|activity| {
                if old_activities.contains(&activity.start_date) {
                    None
                } else {
                    Some(activity.id)
                }
            })
            .map(|activity_id| async move {
                self.export_original(activity_id as u64)
                    .await
                    .map_err(Into::into)
            });
        try_join_all(futures).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct StravaAthlete {
    pub id: u64,
    pub username: StackString,
    pub firstname: StackString,
    pub lastname: StackString,
    pub city: StackString,
    pub state: StackString,
    pub sex: StackString,
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
                    None,
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
        let start_date: DateTime<Utc> = "2020-01-01T00:00:00Z".parse()?;
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

    #[tokio::test]
    #[ignore]
    async fn test_webauth() -> Result<(), Error> {
        use garmin_lib::utils::garmin_util::get_md5sum;
        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::with_auth(config).await?;
        client.webauth().await?;
        client.export_original(3862793062).await?;

        let fname = client
            .config
            .download_directory
            .join("3862793062")
            .with_extension("fit");
        assert!(fname.exists());
        assert_eq!(&get_md5sum(&fname)?, "6365f391e3873cfdfeb5d716195f7271");

        Ok(())
    }

    #[test]
    fn test_extract_web_csrf() -> Result<(), Error> {
        let text = include_str!("../../tests/data/strava_login_page.html");
        let (name, token) = StravaClient::extract_web_csrf(&text)?;
        assert_eq!(name.as_str(), "authenticity_token");
        assert_eq!(
            token.as_str(),
            "1YVkvKYefXvFw1a++rprn9XM1xgT88O6A8UumIH99P4OVYl+wm9GyZp0zBrxNc8hRPqa8wzwJcJ/\
             9YHsQAIZaQ=="
        );
        Ok(())
    }
}
