use anyhow::{format_err, Error};
use base64::{encode, encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
    task::spawn_blocking,
};

use garmin_lib::{
    common::{
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

lazy_static! {
    static ref CSRF_TOKEN: Mutex<Option<StackString>> = Mutex::new(None);
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

    pub async fn from_file(config: GarminConfig) -> Result<Self, Error> {
        let mut client = Self {
            config,
            ..Self::default()
        };
        let f = File::open(client.config.fitbit_tokenfile.as_str()).await?;
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
        client.offset = client.get_client_offset().await.ok();
        Ok(client)
    }

    pub async fn to_file(&self) -> Result<(), Error> {
        let mut f = tokio::fs::File::create(self.config.fitbit_tokenfile.as_str()).await?;
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

    async fn get_client_offset(&self) -> Result<FixedOffset, Error> {
        #[derive(Deserialize)]
        struct UserObj {
            #[serde(rename = "offsetFromUTCMillis")]
            offset: i32,
        }
        #[derive(Deserialize)]
        struct UserResp {
            user: UserObj,
        }

        let headers = self.get_auth_headers()?;
        let url = "https://api.fitbit.com/1/user/-/profile.json";
        let resp: UserResp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let offset = (resp.user.offset / 1000) as i32;
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
            "https://www.fitbit.com/oauth2/authorize",
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
        let url = "https://api.fitbit.com/oauth2/token";
        let auth_resp: AccessTokenResponse = self
            .client
            .post(url)
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
            let url = "https://api.fitbit.com/oauth2/token";
            let auth_resp: AccessTokenResponse = self
                .client
                .post(url)
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
        let url = format!(
            "https://api.fitbit.com/1/user/-/activities/heart/date/{}/1d/1min.json",
            date
        );
        let dataset: HeartRateResp = self
            .client
            .get(url.as_str())
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
        let url = format!(
            "https://api.fitbit.com/1/user/-/body/log/weight/date/{}/30d.json",
            date
        );
        let body_weight: BodyWeight = self
            .client
            .get(url.as_str())
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
                let url = "https://api.fitbit.com/1/user/-/body/log/weight.json";
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

                let url = "https://api.fitbit.com/1/user/-/body/log/fat.json";
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
    ) -> Result<Vec<ActivityEntry>, Error> {
        #[derive(Deserialize)]
        struct AcivityListResp {
            activities: Vec<ActivityEntry>,
        }

        let offset = offset.unwrap_or(0);

        let headers = self.get_auth_headers()?;
        let url = format!(
            "https://api.fitbit.com/1/user/-/activities/list.json?afterDate={}&offset={}&limit=20&sort=asc",
            start_date,
            offset,
        );
        let activities: AcivityListResp = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(activities.activities)
    }

    pub async fn get_all_activities(
        &self,
        start_date: NaiveDate,
    ) -> Result<Vec<ActivityEntry>, Error> {
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
        struct FitbitActivity {
            name: String,
            id: u64,
        }
        #[derive(Deserialize)]
        struct FitbitSubCategory {
            activities: Vec<FitbitActivity>,
            id: u64,
            name: String,
        }
        #[derive(Deserialize)]
        struct FitbitCategory {
            activities: Vec<FitbitActivity>,
            #[serde(rename = "subCategories")]
            sub_categories: Option<Vec<FitbitSubCategory>>,
            id: u64,
            name: String,
        }
        #[derive(Deserialize)]
        struct FitbitActivityCategories {
            categories: Vec<FitbitCategory>,
        }

        let url = "https://api.fitbit.com/1/activities.json";
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

    pub async fn log_fitbit_activity(
        &self,
        entry: &ActivityLoggingEntry,
    ) -> Result<(u64, u64), Error> {
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

        let url = "https://api.fitbit.com/1/user/-/activities.json";
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

    pub async fn sync_fitbit_activities(
        &self,
        begin_datetime: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<DateTime<Utc>>, Error> {
        let offset = self.get_offset();
        let date = begin_datetime.with_timezone(&offset).naive_local().date();
        let new_activities: HashMap<_, _> = self
            .get_all_activities(date)
            .await?
            .into_iter()
            .map(|activity| {
                (
                    activity.start_time.format("%Y-%m-%dT%H:%M").to_string(),
                    activity,
                )
            })
            .collect();

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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActivityEntry {
    #[serde(rename = "logType")]
    log_type: String,
    #[serde(rename = "startTime")]
    start_time: DateTime<Utc>,
    #[serde(rename = "tcxLink")]
    tcx_link: Option<String>,
    #[serde(rename = "activityTypeId")]
    activity_type_id: Option<u64>,
    #[serde(rename = "activityName")]
    activity_name: Option<String>,
    duration: u64,
    distance: Option<f64>,
    #[serde(rename = "distanceUnit")]
    distance_unit: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActivityLoggingEntry {
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
    pub fn from_summary(item: &GarminSummary, offset: FixedOffset) -> Option<Self> {
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

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Duration, Local, Utc};
    use log::debug;
    use std::path::Path;
    use tempfile::NamedTempFile;

    use crate::fitbit_client::FitbitClient;
    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

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
        let client = FitbitClient::from_file(config.clone()).await?;
        let start_date = (Utc::now() - Duration::days(10)).naive_utc().date();
        let results = client.get_tcx_urls(start_date).await?;
        debug!("{:?}", results);
        for (start_time, tcx_url) in results {
            let fname = format!(
                "{}/{}.tcx",
                config.gps_dir,
                start_time
                    .with_timezone(&Local)
                    .format("%Y-%m-%d_%H-%M-%S_1_1")
                    .to_string(),
            );
            if Path::new(&fname).exists() {
                debug!("{} exists", fname);
            } else {
                debug!("{} does not exist", fname);
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
        let client = FitbitClient::from_file(config.clone()).await?;
        let offset = client.offset.unwrap();
        assert_eq!(offset.local_minus_utc(), -4 * 3600);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_fitbit_intraday_time_series_heartrate() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config.clone()).await?;
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
        let client = FitbitClient::from_file(config.clone()).await?;
        let bodyweight = client.get_fitbit_bodyweightfat().await?;
        debug!("{:#?}", bodyweight);
        assert!(bodyweight.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_all_activities() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config.clone()).await?;

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
        let client = FitbitClient::from_file(config.clone()).await?;

        let begin_datetime = Utc::now() - Duration::days(30);

        let pool = PgPool::new(&config.pgurl);
        let dates = client.sync_fitbit_activities(begin_datetime, &pool).await?;
        println!("{:?}", dates);
        assert_eq!(dates.len(), 0);
        Ok(())
    }
}
