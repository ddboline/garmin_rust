use anyhow::{format_err, Error};
use base64::{encode, encode_config, URL_SAFE_NO_PAD};
use crossbeam_utils::atomic::AtomicCell;
use futures::future::try_join_all;
use itertools::Itertools;
use lazy_static::lazy_static;
use log::debug;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Response, Url};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, Duration,
    OffsetDateTime, UtcOffset,
};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    task::spawn_blocking,
    time::sleep,
};

use garmin_connect_lib::garmin_connect_hr_data::GarminConnectHrData;
use garmin_lib::common::{
    fitbit_activity::FitbitActivity,
    garmin_config::GarminConfig,
    garmin_summary::{get_list_of_activities_from_db, GarminSummary},
    pgpool::PgPool,
};

use crate::{
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    scale_measurement::ScaleMeasurement,
};

lazy_static! {
    static ref CSRF_TOKEN: AtomicCell<Option<StackString>> = AtomicCell::new(None);
}

#[derive(Default, Debug, Clone)]
pub struct FitbitClient {
    pub config: GarminConfig,
    pub user_id: StackString,
    pub access_token: StackString,
    pub refresh_token: StackString,
    pub client: Client,
    pub offset: Option<UtcOffset>,
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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    /// Returns error if auth fails
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

    /// # Errors
    /// Returns error if reading auth from file fails
    pub async fn from_file(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self {
            config,
            client: Client::builder().cookie_store(true).build()?,
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

    /// # Errors
    /// Returns error if storing to file fails
    pub async fn to_file(&self) -> Result<(), Error> {
        let mut f = tokio::fs::File::create(&self.config.fitbit_tokenfile).await?;
        f.write_all(format_sstr!("user_id={}\n", self.user_id).as_bytes())
            .await?;
        f.write_all(format_sstr!("access_token={}\n", self.access_token).as_bytes())
            .await?;
        f.write_all(format_sstr!("refresh_token={}\n", self.refresh_token).as_bytes())
            .await?;
        Ok(())
    }

    #[must_use]
    pub fn get_offset(&self) -> UtcOffset {
        self.offset.unwrap_or(UtcOffset::UTC)
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
                    sleep(std::time::Duration::from_secs(retry_seconds)).await;
                    let headers = self.get_auth_headers()?;
                    return self
                        .client
                        .get(url)
                        .headers(headers)
                        .send()
                        .await
                        .map_err(Into::into);
                }
                println!("Wait at least {} seconds before retrying", retry_seconds);
                return Err(format_err!("{}", resp.text().await?));
            }
        }
        Ok(resp)
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_user_profile(&self) -> Result<FitbitUserProfile, Error> {
        #[derive(Deserialize)]
        struct UserResp {
            user: FitbitUserProfile,
        }

        let headers = self.get_auth_headers()?;
        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/user/-/")?
            .join("profile.json")?;
        let resp: UserResp = self
            .get_url(url, headers)
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.user)
    }

    async fn get_client_offset(&self) -> Result<UtcOffset, Error> {
        let profile = self.get_user_profile().await?;
        let offset = (profile.offset_from_utc_millis / 1000) as i32;
        let offset = UtcOffset::from_whole_seconds(offset)?;
        Ok(offset)
    }

    fn get_random_string() -> String {
        let random_bytes: SmallVec<[u8; 16]> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
        encode_config(&random_bytes, URL_SAFE_NO_PAD)
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_fitbit_auth_url(&self) -> Result<Url, Error> {
        let redirect_uri = format_sstr!("https://{}/garmin/fitbit/callback", self.config.domain);
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
        let fitbit_oauth_authorize = self
            .config
            .fitbit_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("oauth2/authorize")?;
        let url = Url::parse_with_params(
            fitbit_oauth_authorize.as_str(),
            &[
                ("response_type", "code"),
                ("client_id", self.config.fitbit_clientid.as_str()),
                ("redirect_url", redirect_uri.as_str()),
                ("scope", scopes.join(" ").as_str()),
                ("state", state.as_str()),
            ],
        )?;
        CSRF_TOKEN.store(Some(state.into()));
        Ok(url)
    }

    fn get_basic_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-type", "application/x-www-form-urlencoded".parse()?);
        headers.insert(
            "Authorization",
            format_sstr!(
                "Basic {}",
                encode(format_sstr!(
                    "{}:{}",
                    self.config.fitbit_clientid,
                    self.config.fitbit_clientsecret
                ))
            )
            .parse()?,
        );
        Ok(headers)
    }

    fn get_auth_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format_sstr!("Bearer {}", self.access_token,).parse()?,
        );
        headers.insert("Accept-Language", "en_US".parse()?);
        headers.insert("Accept-Locale", "en_US".parse()?);
        Ok(headers)
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn refresh_fitbit_access_token(&mut self) -> Result<StackString, Error> {
        let headers = self.get_basic_headers()?;
        let data = hashmap! {
            "grant_type" => "refresh_token",
            "refresh_token" => self.refresh_token.as_str(),
        };
        let fitbit_oauth_token = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("oauth2/token")?;
        let auth_resp: AccessTokenResponse = self
            .client
            .post(fitbit_oauth_token)
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

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_fitbit_access_token(
        &mut self,
        code: &str,
        state: &str,
    ) -> Result<StackString, Error> {
        if let Some(current_state) = CSRF_TOKEN.swap(None) {
            if state != current_state.as_str() {
                return Err(format_err!("Incorrect state"));
            }
            let headers = self.get_basic_headers()?;
            let redirect_uri =
                format_sstr!("https://{}/garmin/fitbit/callback", self.config.domain);
            let data = hashmap! {
                "code" => code,
                "grant_type" => "authorization_code",
                "client_id" => self.config.fitbit_clientid.as_str(),
                "redirect_uri" => redirect_uri.as_str(),
                "state" => state,
            };
            let fitbit_oauth_token = self
                .config
                .fitbit_api_endpoint
                .as_ref()
                .ok_or_else(|| format_err!("Bad URL"))?
                .join("oauth2/token")?;
            let auth_resp: AccessTokenResponse = self
                .client
                .post(fitbit_oauth_token)
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

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_fitbit_intraday_time_series_heartrate(
        &self,
        date: Date,
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
        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/user/-/")?
            .join(&format_sstr!("activities/heart/date/{date}/1d/1min.json"))?;
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
        dataset
            .intraday
            .dataset
            .into_iter()
            .map(|entry| {
                let (h, m, _) = offset.as_hms();
                let offset = format_sstr!(
                    "{s}{h:02}:{m:02}",
                    s = if offset.is_negative() { '-' } else { '+' },
                    h = h.abs(),
                    m = m.abs()
                );
                let datetime = format_sstr!("{date}T{t}{offset}", t = entry.time);
                let datetime = OffsetDateTime::parse(&datetime, &Rfc3339)?.to_timezone(UTC);
                let value = entry.value;
                Ok(FitbitHeartRate { datetime, value })
            })
            .collect()
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn import_fitbit_heartrate(&self, date: Date) -> Result<Vec<FitbitHeartRate>, Error> {
        let heartrates = self.get_fitbit_intraday_time_series_heartrate(date).await?;
        let config = self.config.clone();
        spawn_blocking(move || {
            FitbitHeartRate::merge_slice_to_avro(&config, &heartrates)?;
            Ok(heartrates)
        })
        .await?
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn import_garmin_connect_heartrate(
        config: GarminConfig,
        heartrate_data: &GarminConnectHrData,
    ) -> Result<(), Error> {
        let heartrates = FitbitHeartRate::from_garmin_connect_hr(heartrate_data);
        spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &heartrates)).await?
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_fitbit_bodyweightfat(&self) -> Result<Vec<FitbitBodyWeightFat>, Error> {
        #[derive(Deserialize)]
        struct BodyWeight {
            weight: Vec<WeightEntry>,
        }
        #[derive(Deserialize)]
        struct WeightEntry {
            date: Date,
            fat: Option<f64>,
            time: StackString,
            weight: f64,
        }
        let headers = self.get_auth_headers()?;
        let offset = self.get_offset();
        let date = OffsetDateTime::now_utc().to_offset(offset).date();
        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/user/-/")?
            .join(&format_sstr!("body/log/weight/date/{date}/30d.json"))?;
        let body_weight: BodyWeight = self
            .client
            .get(url)
            .headers(headers.clone())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let result = body_weight
            .weight
            .into_iter()
            .filter_map(|bw| {
                let (h, m, _) = offset.as_hms();
                let offset = format_sstr!(
                    "{s}{h:02}:{m:02}",
                    s = if offset.is_negative() { '-' } else { '+' },
                    h = h.abs(),
                    m = m.abs()
                );
                let datetime = format_sstr!("{d}T{t}{offset}", d = bw.date, t = bw.time);
                let datetime = OffsetDateTime::parse(&datetime, &Rfc3339)
                    .ok()?
                    .to_timezone(UTC);
                let weight = bw.weight;
                let fat = bw.fat?;
                Some(FitbitBodyWeightFat {
                    datetime,
                    weight,
                    fat,
                })
            })
            .collect();
        Ok(result)
    }

    /// # Errors
    /// Returns error if api call fails
    #[allow(clippy::similar_names)]
    pub async fn update_fitbit_bodyweightfat<'a>(
        &self,
        updates: impl IntoIterator<Item = &'a ScaleMeasurement>,
    ) -> Result<(), Error> {
        let headers = self.get_auth_headers()?;
        let offset = self.get_offset();
        let futures = updates.into_iter().map(|update| {
            let headers = headers.clone();
            async move {
                let datetime = update.datetime.to_offset(offset);
                let date = datetime.date();
                let date_str = date
                    .format(format_description!("[year]-[month]-[day]"))
                    .unwrap_or_else(|_| "".into())
                    .into();
                let time_str = date
                    .format(format_description!("[hour]:[minute]:[second]"))
                    .unwrap_or_else(|_| "".into())
                    .into();
                let weight_str = StackString::from_display(update.mass);
                let url = self
                    .config
                    .fitbit_api_endpoint
                    .as_ref()
                    .ok_or_else(|| format_err!("Bad URL"))?
                    .join("1/user/-/")?
                    .join("body/log/weight.json")?;
                let data = hashmap! {
                    "weight" => &weight_str,
                    "date" => &date_str,
                    "time" => &time_str,
                };
                self.client
                    .post(url)
                    .form(&data)
                    .headers(headers.clone())
                    .send()
                    .await?
                    .error_for_status()?;

                let url = self
                    .config
                    .fitbit_api_endpoint
                    .as_ref()
                    .ok_or_else(|| format_err!("Bad URL"))?
                    .join("1/user/-/")?
                    .join("body/log/fat.json")?;
                let fat_pct_str = StackString::from_display(update.fat_pct);
                let data = hashmap! {
                    "fat" => &fat_pct_str,
                    "date" => &date_str,
                    "time" => &time_str,
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
        Ok(())
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_activities(
        &self,
        start_date: Date,
        offset: Option<usize>,
    ) -> Result<Vec<FitbitActivity>, Error> {
        #[derive(Deserialize)]
        struct AcivityListResp {
            activities: Vec<FitbitActivity>,
        }

        let offset = offset.unwrap_or(0);

        let headers = self.get_auth_headers()?;
        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/user/-/")?
            .join(&format_sstr!(
                "activities/list.json?afterDate={start_date}&offset={offset}&limit=20&sort=asc"
            ))?;
        let activities: AcivityListResp = self
            .get_url(url, headers)
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(activities.activities)
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_all_activities(&self, start_date: Date) -> Result<Vec<FitbitActivity>, Error> {
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

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_tcx_urls(
        &self,
        start_date: Date,
    ) -> Result<Vec<(OffsetDateTime, StackString)>, Error> {
        let activities = self.get_activities(start_date, None).await?;

        activities
            .into_iter()
            .filter_map(|entry| {
                let res = || {
                    if entry.log_type != "tracker" {
                        return Ok(None);
                    }
                    let start_time = entry.start_time;
                    entry
                        .tcx_link
                        .map_or(Ok(None), |link| Ok(Some((start_time, link))))
                };
                res().transpose()
            })
            .collect()
    }

    /// # Errors
    /// Returns error if api call fails
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

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_fitbit_activity_types(
        &self,
    ) -> Result<HashMap<StackString, StackString>, Error> {
        #[derive(Deserialize)]
        struct FitbitActivityType {
            name: StackString,
            id: u64,
        }
        #[derive(Deserialize)]
        struct FitbitSubCategory {
            activities: Vec<FitbitActivityType>,
            id: u64,
            name: StackString,
        }
        #[derive(Deserialize)]
        struct FitbitCategory {
            activities: Vec<FitbitActivityType>,
            #[serde(rename = "subCategories")]
            sub_categories: Option<Vec<FitbitSubCategory>>,
            id: u64,
            name: StackString,
        }
        #[derive(Deserialize)]
        struct FitbitActivityCategories {
            categories: Vec<FitbitCategory>,
        }

        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/activities.json")?;
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
        let mut id_map: HashMap<StackString, StackString> = HashMap::new();
        for category in &categories.categories {
            let id_str = StackString::from_display(category.id);
            id_map.insert(id_str, category.name.clone());
            for activity in &category.activities {
                let name = format_sstr!("{}/{}", category.name, activity.name);
                let id_str = StackString::from_display(activity.id);
                id_map.insert(id_str, name);
            }
            if let Some(sub_categories) = category.sub_categories.as_ref() {
                for sub_category in sub_categories.iter() {
                    let name = format_sstr!("{}/{}", category.name, sub_category.name);
                    let id_str = StackString::from_display(sub_category.id);
                    id_map.insert(id_str, name);
                    for sub_activity in &sub_category.activities {
                        let name = format_sstr!(
                            "{}/{}/{}",
                            category.name,
                            sub_category.name,
                            sub_activity.name
                        );
                        let id_str = StackString::from_display(sub_activity.id);
                        id_map.insert(id_str, name);
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

        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .unwrap()
            .join("1/user/-/")?
            .join("activities.json")?;
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

    /// # Errors
    /// Returns error if api call fails
    pub async fn remove_duplicate_entries(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let mut last_entry = None;
        let futures = FitbitActivity::read_from_db(pool, None, None)
            .await?
            .into_iter()
            .map(|activity| {
                let start_time_str = activity
                    .start_time
                    .format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
                    .unwrap_or_else(|_| "".into());
                (start_time_str, activity)
            })
            .sorted_by(|x, y| x.0.cmp(&y.0))
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
            .map(|log_id| async move {
                if self.delete_fitbit_activity(log_id as u64).await.is_err() {
                    debug!("Failed to delete fitbit activity {}", log_id);
                }
                if let Some(activity) = FitbitActivity::get_by_id(pool, log_id).await? {
                    activity.delete_from_db(pool).await?;
                    Ok(format_sstr!("fully deleted {log_id}"))
                } else {
                    Ok(format_sstr!("not fully deleted {log_id}"))
                }
            });
        try_join_all(futures).await
    }

    /// # Errors
    /// Returns error if api call fails
    #[allow(clippy::manual_filter_map)]
    pub async fn sync_fitbit_activities(
        &self,
        begin_datetime: OffsetDateTime,
        pool: &PgPool,
    ) -> Result<Vec<OffsetDateTime>, Error> {
        let offset = self.get_offset();
        let date = begin_datetime.to_offset(offset).date();

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
                if self.delete_fitbit_activity(*log_id as u64).await.is_err() {
                    debug!("Failed to delete fitbit activity {}", log_id);
                }
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
                    let start_time_str = activity
                        .start_time
                        .format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
                        .unwrap_or_else(|_| "".into());
                    Some((start_time_str, activity))
                }
            })
            .collect();

        // Get existing activities
        let existing_activities: HashMap<_, _> =
            FitbitActivity::read_from_db(pool, Some(date), None)
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

        let old_activities = get_list_of_activities_from_db(
            &format_sstr!("begin_datetime >= '{begin_datetime}'"),
            pool,
        )
        .await?
        .into_iter()
        .filter(|(d, _)| {
            let key = d
                .format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
                .unwrap_or_else(|_| "".into());
            !new_activities.contains_key(&key)
        });

        let futures = old_activities.map(|(d, f)| {
            let pool = pool.clone();
            async move {
                if let Some(activity) = GarminSummary::read_summary_from_postgres(&pool, &f)
                    .await?
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
        let updated: Result<Vec<_>, Error> = try_join_all(futures).await;
        Ok(updated?.into_iter().flatten().collect())
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn delete_fitbit_activity(&self, log_id: u64) -> Result<(), Error> {
        let url = self
            .config
            .fitbit_api_endpoint
            .as_ref()
            .ok_or_else(|| format_err!("Bad URL"))?
            .join("1/user/-/")?
            .join(&format_sstr!("activities/{log_id}.json"))?;
        let headers = self.get_auth_headers()?;
        self.client
            .delete(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn sync_everything(
        &self,
        pool: &PgPool,
    ) -> Result<FitbitBodyWeightFatUpdateOutput, Error> {
        let offset = self.get_offset();
        let start_datetime = OffsetDateTime::now_utc() - Duration::days(30);
        let start_date: Date = start_datetime.to_offset(offset).date();
        let local = time_tz::system::get_timezone()?;

        let existing_map: HashMap<_, _> = self
            .get_fitbit_bodyweightfat()
            .await?
            .into_iter()
            .map(|entry| {
                let date = entry.datetime.to_timezone(local).date();
                (date, entry)
            })
            .collect();

        let measurements = ScaleMeasurement::read_from_db(pool, Some(start_date), None)
            .await?
            .into_iter()
            .filter(|entry| {
                let date = entry.datetime.to_timezone(local).date();
                !existing_map.contains_key(&date)
            })
            .collect();
        self.update_fitbit_bodyweightfat(&measurements).await?;

        let activities: Vec<_> = self
            .sync_fitbit_activities(start_datetime, pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        let duplicates = self.remove_duplicate_entries(pool).await?;
        FitbitActivity::fix_summary_id_in_db(pool).await?;
        if !activities.is_empty() {
            self.sync_fitbit_activities(start_datetime, pool).await?;
            self.remove_duplicate_entries(pool).await?;
            FitbitActivity::fix_summary_id_in_db(pool).await?;
        }

        Ok(FitbitBodyWeightFatUpdateOutput {
            measurements,
            activities,
            duplicates,
        })
    }

    /// # Errors
    /// Returns error if api call fails
    #[allow(clippy::manual_filter_map)]
    pub async fn sync_tcx(&self, start_date: Date) -> Result<Vec<PathBuf>, Error> {
        let futures = self
            .get_tcx_urls(start_date)
            .await?
            .into_iter()
            .filter_map(|(start_time, tcx_url)| {
                let fname = self
                    .config
                    .gps_dir
                    .join(
                        start_time
                            .format(format_description!(
                                "[year]-[month]-[day]_[hour]-[minute]-[second]_1_1"
                            ))
                            .unwrap_or_else(|_| "".into()),
                    )
                    .with_extension("tcx");
                if fname.exists() {
                    None
                } else {
                    Some((fname, tcx_url))
                }
            })
            .map(|(fname, tcx_url)| async move {
                let data = self.download_tcx(&tcx_url).await?;
                tokio::fs::write(&fname, &data).await?;
                Ok(fname)
            });
        try_join_all(futures).await
    }
}

#[derive(Debug, Serialize)]
pub struct FitbitBodyWeightFatUpdateOutput {
    pub measurements: Vec<ScaleMeasurement>,
    pub activities: Vec<OffsetDateTime>,
    pub duplicates: Vec<StackString>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ActivityLoggingEntry {
    #[serde(rename = "activityId")]
    activity_id: Option<u64>,
    #[serde(rename = "startTime")]
    start_time: StackString,
    #[serde(rename = "durationMillis")]
    duration_millis: u64,
    date: Date,
    distance: Option<f64>,
    #[serde(rename = "distanceUnit")]
    distance_unit: Option<StackString>,
}

impl ActivityLoggingEntry {
    fn from_summary(item: &GarminSummary, offset: UtcOffset) -> Option<Self> {
        let start_time_str = item
            .begin_datetime
            .to_offset(offset)
            .format(format_description!("[hour]:[minute]"))
            .ok()?
            .into();
        item.sport.to_fitbit_activity_id().map(|activity_id| Self {
            activity_id: Some(activity_id),
            start_time: start_time_str,
            duration_millis: (item.total_duration * 1000.0) as u64,
            date: item.begin_datetime.to_offset(offset).date(),
            distance: Some(item.total_distance / 1000.0),
            distance_unit: Some("Kilometer".into()),
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FitbitUserProfile {
    #[serde(alias = "averageDailySteps")]
    pub average_daily_steps: u64,
    pub country: StackString,
    #[serde(alias = "dateOfBirth")]
    pub date_of_birth: StackString,
    #[serde(alias = "displayName")]
    pub display_name: StackString,
    #[serde(alias = "distanceUnit")]
    pub distance_unit: StackString,
    #[serde(alias = "encodedId")]
    pub encoded_id: StackString,
    #[serde(alias = "firstName")]
    pub first_name: StackString,
    #[serde(alias = "lastName")]
    pub last_name: StackString,
    #[serde(alias = "fullName")]
    pub full_name: StackString,
    pub gender: StackString,
    pub height: f64,
    #[serde(alias = "heightUnit")]
    pub height_unit: StackString,
    pub timezone: StackString,
    #[serde(alias = "offsetFromUTCMillis")]
    pub offset_from_utc_millis: i64,
    #[serde(alias = "strideLengthRunning")]
    pub stride_length_running: f64,
    #[serde(alias = "strideLengthWalking")]
    pub stride_length_walking: f64,
    pub weight: f64,
    #[serde(alias = "weightUnit")]
    pub weight_unit: StackString,
}

#[cfg(test)]
mod tests {
    use crate::fitbit_client::{FitbitActivity, FitbitClient};
    use anyhow::Error;
    use futures::future::try_join_all;
    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};
    use log::debug;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;
    use time::{
        macros::{date, format_description},
        Date, Duration, OffsetDateTime,
    };
    use time_tz::OffsetDateTimeExt;

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
        let local = time_tz::system::get_timezone()?;
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let start_date = (OffsetDateTime::now_utc() - Duration::days(10)).date();
        let results = client.get_tcx_urls(start_date).await?;
        debug!("{:?}", results);
        for (start_time, tcx_url) in results {
            let fname = config
                .gps_dir
                .join(
                    start_time
                        .to_timezone(local)
                        .format(format_description!(
                            "[year]-[month]-[day]_[hour]-[minute]-[second]_1_1"
                        ))
                        .unwrap(),
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
                debug!("{start_time} {metadata:?} {l}", l = metadata.len());
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
        assert!(offset.whole_seconds() == -5 * 3600 || offset.whole_seconds() == -4 * 3600);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_fitbit_intraday_time_series_heartrate() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::with_auth(config.clone()).await?;
        let date = (OffsetDateTime::now_utc() - Duration::days(1)).date();
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
        let begin_datetime = (OffsetDateTime::now_utc() - Duration::days(7)).to_offset(offset);

        let date = begin_datetime.date();
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

        let begin_datetime = OffsetDateTime::now_utc() - Duration::days(30);

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
        let start_date: Date = date!(2020 - 01 - 01);
        let new_activities: Vec<_> = client
            .get_all_activities(start_date)
            .await?
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.log_id))
            .collect();
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
