use anyhow::{format_err, Error};
use base64::{encode, encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use futures::future::try_join_all;
use itertools::Itertools;
use lazy_static::lazy_static;
use log::debug;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Response, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
    task::spawn_blocking,
    time::{delay_for, Duration},
};

use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_config::GarminConfig,
        garmin_connect_client::GarminConnectClient,
        garmin_summary::{get_list_of_activities_from_db, GarminSummary, GarminSummaryList},
        pgpool::PgPool,
    },
    utils::stack_string::StackString,
};

use crate::{
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    scale_measurement::ScaleMeasurement,
};

const FITBIT_OAUTH_AUTHORIZE: &str = "https://www.fitbit.com/oauth2/authorize";
const FITBIT_OAUTH_TOKEN: &str = "https://api.fitbit.com/oauth2/token";

lazy_static! {
    static ref CSRF_TOKEN: Mutex<Option<StackString>> = Mutex::new(None);
    static ref FITBIT_ENDPOINT: Url = Url::parse("https://api.fitbit.com/").unwrap();
    static ref FITBIT_PREFIX: Url = Url::parse("https://api.fitbit.com/1/user/-/").unwrap();
}

#[derive(Default, Debug, Clone)]
pub struct FitbitClient {
    pub config: GarminConfig,
    pub user_id: StackString,
    pub access_token: StackString,
    pub refresh_token: StackString,
    pub client: Client,
    pub offset: Option<FixedOffset>,
}

#[derive(Serialize, Deserialize, Debug)]
struct AccessTokenResponse {
    access_token: StackString,
    token_type: StackString,
    expires_in: u64,
    refresh_token: StackString,
    user_id: StackString,
}

impl FitbitClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn with_auth(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self::from_file(config).await?;
        if let Ok(offset) = client.get_client_offset().await {
            client.offset = Some(offset);
        } else {
            let body = client.refresh_fitbit_access_token().await?;
            debug!("{}", body);
            client.to_file().await?;
            client.offset = Some(client.get_client_offset().await?);
        }
        Ok(client)
    }

    pub async fn from_file(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self {
            config,
            ..Self::default()
        };
        let f = File::open(&client.config.fitbit_tokenfile).await?;
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
                    match key {
                        "user_id" => client.user_id = val.trim().into(),
                        "access_token" => client.access_token = val.trim().into(),
                        "refresh_token" => client.refresh_token = val.trim().into(),
                        _ => {}
                    }
                }
            }
        }
        Ok(client)
    }

    pub async fn to_file(&self) -> Result<(), Error> {
        let mut f = tokio::fs::File::create(&self.config.fitbit_tokenfile).await?;
        f.write_all(format!("user_id={}\n", self.user_id).as_bytes())
            .await?;
        f.write_all(format!("access_token={}\n", self.access_token).as_bytes())
            .await?;
        f.write_all(format!("refresh_token={}\n", self.refresh_token).as_bytes())
            .await?;
        Ok(())
    }

    pub fn get_offset(&self) -> FixedOffset {
        self.offset.unwrap_or_else(|| FixedOffset::east(0))
    }

    async fn get_url(&self, url: Url, headers: HeaderMap) -> Result<Response, Error> {
        let resp = self
            .client
            .get(url.clone())
            .headers(headers.clone())
            .send()
            .await?;
        if resp.status() == 429 {
            if let Some(retry_after) = resp.headers().get("retry-after") {
                let retry_seconds: u64 = retry_after.to_str()?.parse()?;
                if retry_seconds < 60 {
                    delay_for(Duration::from_secs(retry_seconds)).await;
                    let headers = self.get_auth_headers()?;
                    return self
                        .client
                        .get(url)
                        .headers(headers)
                        .send()
                        .await
                        .map_err(Into::into);
                } else {
                    println!("Wait at least {} seconds before retrying", retry_seconds);
                    return Err(format_err!("{}", resp.text().await?));
                }
            }
        }
        Ok(resp)
    }

    pub async fn get_user_profile(&self) -> Result<FitbitUserProfile, Error> {
        #[derive(Deserialize)]
        struct UserResp {
            user: FitbitUserProfile,
        }

        let headers = self.get_auth_headers()?;
        let url = FITBIT_PREFIX.join("profile.json")?;
        let resp: UserResp = self
            .get_url(url, headers)
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.user)
    }

    async fn get_client_offset(&self) -> Result<FixedOffset, Error> {
        let profile = self.get_user_profile().await?;
        let offset = (profile.offset_from_utc_millis / 1000) as i32;
        let offset = FixedOffset::east(offset);
        Ok(offset)
    }

    fn get_random_string() -> String {
        let random_bytes: Vec<u8> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD)
    }

    pub async fn get_fitbit_auth_url(&self) -> Result<Url, Error> {
        let redirect_uri = format!("https://{}/garmin/fitbit/callback", self.config.domain);
        let scopes = &[
            "activity",
            "nutrition",
            "heartrate",
            "location",
            "profile",
            "settings",
            "sleep",
            "social",
            "weight",
        ];
        let state = Self::get_random_string();
        let url = Url::parse_with_params(
            FITBIT_OAUTH_AUTHORIZE,
            &[
                ("response_type", "code"),
                ("client_id", self.config.fitbit_clientid.as_str()),
                ("redirect_url", redirect_uri.as_str()),
                ("scope", scopes.join(" ").as_str()),
                ("state", state.as_str()),
            ],
        )?;
        CSRF_TOKEN.lock().await.replace(state.into());
        Ok(url)
    }

    fn get_basic_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-type", "application/x-www-form-urlencoded".parse()?);
        headers.insert(
            "Authorization",
            format!(
                "Basic {}",
                encode(format!(
                    "{}:{}",
                    self.config.fitbit_clientid, self.config.fitbit_clientsecret
                ))
            )
            .parse()?,
        );
        headers.insert("trakt-api-version", "2".parse()?);
        Ok(headers)
    }

    fn get_auth_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.access_token,).parse()?,
        );
        headers.insert("Accept-Language", "en_US".parse()?);
        headers.insert("Accept-Locale", "en_US".parse()?);
        Ok(headers)
    }

    pub async fn refresh_fitbit_access_token(&mut self) -> Result<StackString, Error> {
        let headers = self.get_basic_headers()?;
        let data = hashmap! {
            "grant_type" => "refresh_token",
            "refresh_token" => self.refresh_token.as_str(),
        };
        let auth_resp: AccessTokenResponse = self
            .client
            .post(FITBIT_OAUTH_TOKEN)
            .headers(headers)
            .form(&data)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        self.user_id = auth_resp.user_id;
        self.access_token = auth_resp.access_token;
        self.refresh_token = auth_resp.refresh_token;
        let success = r#"
            <h1>You are now authorized to access the Fitbit API!</h1>
            <br/><h3>You can close this window</h3>
            <script language="JavaScript" type="text/javascript">window.close()</script>
            "#
        .into();
        Ok(success)
    }

    pub async fn get_fitbit_access_token(
        &mut self,
        code: &str,
        state: &str,
    ) -> Result<String, Error> {
        let current_state = CSRF_TOKEN.lock().await.take();
        if let Some(current_state) = current_state {
            if state != current_state.as_str() {
                return Err(format_err!("Incorrect state"));
            }
            let headers = self.get_basic_headers()?;
            let redirect_uri = format!("https://{}/garmin/fitbit/callback", self.config.domain);
            let data = hashmap! {
                "code" => code,
                "grant_type" => "authorization_code",
                "client_id" => self.config.fitbit_clientid.as_str(),
                "redirect_uri" => redirect_uri.as_str(),
                "state" => state,
            };
            let auth_resp: AccessTokenResponse = self
                .client
                .post(FITBIT_OAUTH_TOKEN)
                .headers(headers)
                .form(&data)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            self.user_id = auth_resp.user_id;
            self.access_token = auth_resp.access_token;
            self.refresh_token = auth_resp.refresh_token;
            let success = r#"
                <h1>You are now authorized to access the Fitbit API!</h1>
                <br/><h3>You can close this window</h3>
                <script language="JavaScript" type="text/javascript">window.close()</script>
                "#
            .into();
            Ok(success)
        } else {
            Err(format_err!("No state"))
        }
    }

    pub async fn get_fitbit_intraday_time_series_heartrate(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<FitbitHeartRate>, Error> {
        #[derive(Deserialize)]
        struct HeartRateResp {
            #[serde(rename = "activities-heart-intraday")]
            intraday: HrDs,
        }
        #[derive(Deserialize)]
        struct HrDs {
            dataset: Vec<HrDataSet>,
        }
        #[derive(Deserialize)]
        struct HrDataSet {
            time: StackString,
            value: i32,
        }

        let headers = self.get_auth_headers()?;
        let url = FITBIT_PREFIX.join(&format!("activities/heart/date/{}/1d/1min.json", date))?;
        let dataset: HeartRateResp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let offset = self.get_offset();
        let hr_values: Vec<_> = dataset
            .intraday
            .dataset
            .into_iter()
            .map(|entry| {
                let datetime = format!("{}T{}{}", date, entry.time, offset);
                let datetime = DateTime::parse_from_rfc3339(&datetime)
                    .unwrap()
                    .with_timezone(&Utc);
                let value = entry.value;
                FitbitHeartRate { datetime, value }
            })
            .collect();
        Ok(hr_values)
    }

    pub async fn import_fitbit_heartrate(
        &self,
        date: NaiveDate,
        config: &GarminConfig,
    ) -> Result<(), Error> {
        let heartrates = self.get_fitbit_intraday_time_series_heartrate(date).await?;
        let config = config.clone();
        spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &heartrates)).await?
    }

    pub async fn import_garmin_connect_heartrate(
        date: NaiveDate,
        session: &GarminConnectClient,
    ) -> Result<(), Error> {
        let heartrates =
            FitbitHeartRate::from_garmin_connect_hr(&session.get_heartrate(date).await?);
        let config = session.config.clone();
        spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &heartrates)).await?
    }

    pub async fn get_fitbit_bodyweightfat(&self) -> Result<Vec<FitbitBodyWeightFat>, Error> {
        #[derive(Deserialize)]
        struct BodyWeight {
            weight: Vec<WeightEntry>,
        }
        #[derive(Deserialize)]
        struct WeightEntry {
            date: NaiveDate,
            fat: f64,
            time: NaiveTime,
            weight: f64,
        }
        let headers = self.get_auth_headers()?;
        let offset = self.get_offset();
        let date = Utc::now().with_timezone(&offset).date().naive_local();
        let url = FITBIT_PREFIX.join(&format!("body/log/weight/date/{}/30d.json", date))?;
        let body_weight: BodyWeight = self
            .client
            .get(url)
            .headers(headers.clone())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let result: Vec<_> = body_weight
            .weight
            .into_iter()
            .map(|bw| {
                let datetime = format!("{}T{}{}", bw.date, bw.time, offset);
                let datetime = DateTime::parse_from_rfc3339(&datetime)
                    .unwrap()
                    .with_timezone(&Utc);
                let weight = bw.weight;
                let fat = bw.fat;
                FitbitBodyWeightFat {
                    datetime,
                    weight,
                    fat,
                }
            })
            .collect();
        Ok(result)
    }

    #[allow(clippy::similar_names)]
    pub async fn update_fitbit_bodyweightfat(
        &self,
        updates: Vec<ScaleMeasurement>,
    ) -> Result<Vec<ScaleMeasurement>, Error> {
        let headers = self.get_auth_headers()?;
        let offset = self.get_offset();
        let futures = updates.iter().map(|update| {
            let headers = headers.clone();
            async move {
                let datetime = update.datetime.with_timezone(&offset);
                let date = datetime.date().naive_local();
                let time = datetime.naive_local().format("%H:%M:%S").to_string();
                let url = FITBIT_PREFIX.join("body/log/weight.json")?;
                let data = hashmap! {
                    "weight" => update.mass.to_string(),
                    "date" => date.to_string(),
                    "time" => time.to_string(),
                };
                self.client
                    .post(url)
                    .form(&data)
                    .headers(headers.clone())
                    .send()
                    .await?
                    .error_for_status()?;

                let url = FITBIT_PREFIX.join("body/log/fat.json")?;
                let data = hashmap! {
                    "fat" => update.fat_pct.to_string(),
                    "date" => date.to_string(),
                    "time" => time.to_string(),
                };
                self.client
                    .post(url)
                    .form(&data)
                    .headers(headers)
                    .send()
                    .await?
                    .error_for_status()?;
                Ok(())
            }
        });
        let result: Result<Vec<_>, Error> = try_join_all(futures).await;
        result?;
        Ok(updates)
    }

    pub async fn get_activities(
        &self,
        start_date: NaiveDate,
        offset: Option<usize>,
    ) -> Result<Vec<FitbitActivity>, Error> {
        #[derive(Deserialize)]
        struct AcivityListResp {
            activities: Vec<FitbitActivity>,
        }

        let offset = offset.unwrap_or(0);

        let headers = self.get_auth_headers()?;
        let url = FITBIT_PREFIX.join(&format!(
            "activities/list.json?afterDate={}&offset={}&limit=20&sort=asc",
            start_date, offset,
        ))?;
        let activities: AcivityListResp = self
            .get_url(url, headers)
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(activities.activities)
    }

    pub async fn get_all_activities(
        &self,
        start_date: NaiveDate,
    ) -> Result<Vec<FitbitActivity>, Error> {
        let mut activities = Vec::new();
        loop {
            let new_activities: Vec<_> = self
                .get_activities(start_date, Some(activities.len()))
                .await?;
            if new_activities.is_empty() {
                break;
            }
            activities.extend_from_slice(&new_activities);
        }
        Ok(activities)
    }

    pub async fn get_tcx_urls(
        &self,
        start_date: NaiveDate,
    ) -> Result<Vec<(DateTime<Utc>, String)>, Error> {
        let activities = self.get_activities(start_date, None).await?;

        activities
            .into_iter()
            .filter_map(|entry| {
                let res = || {
                    if entry.log_type != "tracker" {
                        return Ok(None);
                    }
                    let start_time = entry.start_time;
                    if let Some(link) = entry.tcx_link {
                        Ok(Some((start_time, link)))
                    } else {
                        Ok(None)
                    }
                };
                res().transpose()
            })
            .collect()
    }

    pub async fn download_tcx(&self, tcx_url: &str) -> Result<bytes::Bytes, Error> {
        let headers = self.get_auth_headers()?;
        self.client
            .get(tcx_url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .map_err(Into::into)
    }

    pub async fn get_fitbit_activity_types(&self) -> Result<HashMap<u64, String>, Error> {
        #[derive(Deserialize)]
        struct FitbitActivityType {
            name: String,
            id: u64,
        }
        #[derive(Deserialize)]
        struct FitbitSubCategory {
            activities: Vec<FitbitActivityType>,
            id: u64,
            name: String,
        }
        #[derive(Deserialize)]
        struct FitbitCategory {
            activities: Vec<FitbitActivityType>,
            #[serde(rename = "subCategories")]
            sub_categories: Option<Vec<FitbitSubCategory>>,
            id: u64,
            name: String,
        }
        #[derive(Deserialize)]
        struct FitbitActivityCategories {
            categories: Vec<FitbitCategory>,
        }

        let url = FITBIT_ENDPOINT.join("1/activities.json")?;
        let headers = self.get_auth_headers()?;
        let categories: FitbitActivityCategories = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut id_map: HashMap<u64, String> = HashMap::new();
        for category in &categories.categories {
            id_map.insert(category.id, category.name.to_string());
            for activity in &category.activities {
                let name = format!("{}/{}", category.name, activity.name);
                id_map.insert(activity.id, name);
            }
            if let Some(sub_categories) = category.sub_categories.as_ref() {
                for sub_category in sub_categories.iter() {
                    let name = format!("{}/{}", category.name, sub_category.name);
                    id_map.insert(sub_category.id, name);
                    for sub_activity in &sub_category.activities {
                        let name = format!(
                            "{}/{}/{}",
                            category.name, sub_category.name, sub_activity.name
                        );
                        id_map.insert(sub_activity.id, name);
                    }
                }
            }
        }
        Ok(id_map)
    }

    async fn log_fitbit_activity(&self, entry: &ActivityLoggingEntry) -> Result<(u64, u64), Error> {
        #[derive(Deserialize)]
        struct ActivityLogEntry {
            #[serde(rename = "activityId")]
            activity_id: u64,
            steps: u64,
        }

        #[derive(Deserialize)]
        struct ActivityLogResp {
            #[serde(rename = "activityLog")]
            activity_log: ActivityLogEntry,
        }

        let url = FITBIT_PREFIX.join("activities.json")?;
        let headers = self.get_auth_headers()?;
        let resp: ActivityLogResp = self
            .client
            .post(url)
            .headers(headers)
            .form(entry)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok((resp.activity_log.activity_id, resp.activity_log.steps))
    }

    pub async fn remove_duplicate_entries(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
        let existing_activities: Vec<_> = FitbitActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| {
                (
                    activity.start_time.format("%Y-%m-%dT%H:%M").to_string(),
                    activity,
                )
            })
            .sorted_by(|x, y| x.0.cmp(&y.0))
            .collect();

        let mut last_entry = None;
        let dupes: Vec<_> = existing_activities
            .into_iter()
            .filter_map(|(k, v)| {
                let mut keep = false;
                if let Some(k_) = last_entry.take() {
                    if k_ == k {
                        keep = true;
                    }
                }
                last_entry.replace(k);
                if keep {
                    Some(v.log_id)
                } else {
                    None
                }
            })
            .collect();

        let futures = dupes.into_iter().map(|log_id| async move {
            self.delete_fitbit_activity(log_id as u64).await?;
            if let Some(activity) = FitbitActivity::get_by_id(&pool, log_id).await? {
                activity.delete_from_db(&pool).await?;
                Ok(format!("fully deleted {}", log_id))
            } else {
                Ok(format!("not fully deleted {}", log_id))
            }
        });
        try_join_all(futures).await
    }

    #[allow(clippy::filter_map)]
    pub async fn sync_fitbit_activities(
        &self,
        begin_datetime: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<DateTime<Utc>>, Error> {
        let offset = self.get_offset();
        let date = begin_datetime.with_timezone(&offset).naive_local().date();

        // Get all activities
        let new_activities: Vec<_> = self.get_all_activities(date).await?;

        // Get id's for walking and running activities with 0 steps
        let activities_to_delete: HashSet<_> = new_activities
            .iter()
            .filter_map(|activity| {
                if (activity.steps.unwrap_or(0) == 0)
                    && (activity.activity_type_id == Some(90009)
                        || activity.activity_type_id == Some(90013))
                {
                    Some(activity.log_id)
                } else {
                    None
                }
            })
            .collect();

        // delete 0 step activities from fitbit and DB
        let futures = activities_to_delete.iter().map(|log_id| {
            let pool = pool.clone();
            async move {
                self.delete_fitbit_activity(*log_id as u64).await?;
                if let Some(activity) = FitbitActivity::get_by_id(&pool, *log_id).await? {
                    activity.delete_from_db(&pool).await?;
                }
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;

        let new_activities: HashMap<_, _> = new_activities
            .into_iter()
            .filter_map(|activity| {
                if activities_to_delete.contains(&activity.log_id) {
                    None
                } else {
                    Some((
                        activity.start_time.format("%Y-%m-%dT%H:%M").to_string(),
                        activity,
                    ))
                }
            })
            .collect();

        // Get existing activities
        let existing_activities: HashMap<_, _> =
            FitbitActivity::read_from_db(&pool, Some(date), None)
                .await?
                .into_iter()
                .map(|activity| (activity.log_id, activity))
                .collect();

        let futures = new_activities
            .values()
            .filter(|activity| !existing_activities.contains_key(&activity.log_id))
            .map(|activity| {
                let pool = pool.clone();
                async move {
                    activity.insert_into_db(&pool).await?;
                    Ok(())
                }
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;

        let old_activities: Vec<_> = get_list_of_activities_from_db(
            &format!("begin_datetime >= '{}'", begin_datetime),
            &pool,
        )
        .await?
        .into_iter()
        .filter(|(d, _)| {
            let key = d.format("%Y-%m-%dT%H:%M").to_string();
            !new_activities.contains_key(&key)
        })
        .collect();

        let summary = Arc::new(GarminSummaryList::new(pool));

        let futures = old_activities.into_iter().map(|(d, f)| {
            let summary = summary.clone();
            async move {
                if let Some(activity) = summary
                    .read_summary_from_postgres(&f)
                    .await?
                    .summary_list
                    .pop()
                {
                    if let Some(activity) = ActivityLoggingEntry::from_summary(&activity, offset) {
                        self.log_fitbit_activity(&activity).await?;
                        return Ok(Some(d));
                    }
                }
                Ok(None)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let updated: Vec<_> = results?.into_iter().filter_map(|x| x).collect();
        Ok(updated)
    }

    pub async fn delete_fitbit_activity(&self, log_id: u64) -> Result<(), Error> {
        let url = FITBIT_PREFIX.join(&format!("activities/{}.json", log_id))?;
        let headers = self.get_auth_headers()?;
        self.client
            .delete(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ActivityLoggingEntry {
    #[serde(rename = "activityId")]
    activity_id: Option<u64>,
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(rename = "durationMillis")]
    duration_millis: u64,
    date: NaiveDate,
    distance: Option<f64>,
    #[serde(rename = "distanceUnit")]
    distance_unit: Option<String>,
}

impl ActivityLoggingEntry {
    fn from_summary(item: &GarminSummary, offset: FixedOffset) -> Option<Self> {
        item.sport.to_fitbit_activity_id().map(|activity_id| Self {
            activity_id: Some(activity_id),
            start_time: item
                .begin_datetime
                .with_timezone(&offset)
                .format("%H:%M")
                .to_string(),
            duration_millis: (item.total_duration * 1000.0) as u64,
            date: item
                .begin_datetime
                .with_timezone(&offset)
                .date()
                .naive_local(),
            distance: Some(item.total_distance / 1000.0),
            distance_unit: Some("Kilometer".to_string()),
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FitbitUserProfile {
    #[serde(rename = "averageDailySteps")]
    pub average_daily_steps: u64,
    pub country: String,
    #[serde(rename = "dateOfBirth")]
    pub date_of_birth: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "distanceUnit")]
    pub distance_unit: String,
    #[serde(rename = "encodedId")]
    pub encoded_id: String,
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub gender: String,
    pub height: f64,
    #[serde(rename = "heightUnit")]
    pub height_unit: String,
    pub timezone: String,
    #[serde(rename = "offsetFromUTCMillis")]
    pub offset_from_utc_millis: i64,
    #[serde(rename = "strideLengthRunning")]
    pub stride_length_running: f64,
    #[serde(rename = "strideLengthWalking")]
    pub stride_length_walking: f64,
    pub weight: f64,
    #[serde(rename = "weightUnit")]
    pub weight_unit: String,
}

#[cfg(test)]
mod tests {
    use crate::fitbit_client::{FitbitActivity, FitbitClient};
    use anyhow::Error;
    use chrono::{Duration, Local, NaiveDate, Utc};
    use futures::future::try_join_all;
    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};
    use log::debug;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    #[tokio::test]
    #[ignore]
    async fn test_fitbit_client_from_file() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config).await?;
        let url = client.get_fitbit_auth_url().await?;
        debug!("{:?} {}", client, url);
        assert!(url.as_str().len() > 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_tcx_urls() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
        let results = client.get_tcx_urls(start_date).await?;
        debug!("{:?}", results);
        for (start_time, tcx_url) in results {
            let fname = config
                .gps_dir
                .join(
                    start_time
                        .with_timezone(&Local)
                        .format("%Y-%m-%d_%H-%M-%S_1_1")
                        .to_string(),
                )
                .with_extension("tcx");
            if fname.exists() {
                debug!("{:?} exists", fname);
            } else {
                debug!("{:?} does not exist", fname);
            }

            {
                use std::io::Write;
                let mut f = NamedTempFile::new()?;
                let data = client.download_tcx(&tcx_url).await?;
                f.write_all(&data)?;

                let metadata = f.as_file().metadata()?;
                debug!("{} {:?} {}", start_time, metadata, metadata.len());
                assert!(metadata.len() > 0);
            }
            break;
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_client_offset() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let offset = client.offset.unwrap();
        assert_eq!(offset.local_minus_utc(), -4 * 3600);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_fitbit_intraday_time_series_heartrate() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let date = (Utc::now() - Duration::days(1)).naive_local().date();
        let heartrates = client
            .get_fitbit_intraday_time_series_heartrate(date)
            .await?;
        debug!("{:#?}", heartrates);
        assert!(heartrates.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_fitbit_bodyweightfat() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let bodyweight = client.get_fitbit_bodyweightfat().await?;
        debug!("{:#?}", bodyweight);
        assert!(bodyweight.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_all_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;

        let offset = client.get_offset();
        let begin_datetime = (Utc::now() - Duration::days(7)).with_timezone(&offset);

        let date = begin_datetime.naive_local().date();
        let new_activities = client.get_all_activities(date).await?;
        println!("{:#?}", new_activities);
        assert!(new_activities.len() > 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_sync_fitbit_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;

        let begin_datetime = Utc::now() - Duration::days(30);

        let pool = PgPool::new(&config.pgurl);
        let dates = client.sync_fitbit_activities(begin_datetime, &pool).await?;
        println!("{:?}", dates);
        assert_eq!(dates.len(), 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_user_profile() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let resp = client.get_user_profile().await?;
        assert_eq!(resp.country.as_str(), "US");
        assert_eq!(resp.display_name.as_str(), "Daniel B.");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_dump_fitbit_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let pool = PgPool::new(&config.pgurl);
        let activities: HashMap<_, _> = FitbitActivity::read_from_db(&pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.log_id, activity))
            .collect();
        let start_date: NaiveDate = "2020-01-01".parse()?;
        let new_activities: Vec<_> = client
            .get_all_activities(start_date)
            .await?
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.log_id))
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
    async fn test_delete_duplicate_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let client = FitbitClient::with_auth(config.clone()).await?;

        let output = client.remove_duplicate_entries(&pool).await?;
        println!("{:?}", output);
        assert_eq!(output.len(), 0);
        Ok(())
    }
}
