use anyhow::{format_err, Error};
use base64::{encode, encode_config, URL_SAFE_NO_PAD};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use maplit::hashmap;
use rand::{thread_rng, Rng};
use reqwest::{header::HeaderMap, Client, Url};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
    task::spawn_blocking,
};

use garmin_lib::{
    common::{garmin_config::GarminConfig, garmin_connect_client::GarminConnectClient},
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

    async fn get_client_offset(&self) -> Result<FixedOffset, Error> {
        #[derive(Deserialize)]
        struct UserObj {
            #[serde(alias = "offsetFromUTCMillis")]
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
            #[serde(alias = "activities-heart-intraday")]
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
        let offset = self.get_client_offset().await?;
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
        let date = Utc::now().naive_local().date();
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
        let offset = self.get_client_offset().await?;
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
        let offset = self.get_client_offset().await?;
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

    pub async fn get_tcx_urls(
        &self,
        start_date: NaiveDate,
    ) -> Result<Vec<(DateTime<Utc>, String)>, Error> {
        #[derive(Deserialize)]
        struct AcivityListResp {
            activities: Vec<ActivityEntry>,
        }
        #[derive(Deserialize)]
        struct ActivityEntry {
            #[serde(alias = "logType")]
            log_type: String,
            #[serde(alias = "startTime")]
            start_time: String,
            #[serde(alias = "tcxLink")]
            tcx_link: Option<String>,
        }

        let headers = self.get_auth_headers()?;
        let url = format!(
            "https://api.fitbit.com/1/user/-/activities/list.json?afterDate={}&offset=0&limit=20&sort=asc",
            start_date,
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
        activities
            .activities
            .into_iter()
            .filter_map(|entry| {
                let res = || {
                    if entry.log_type != "tracker" {
                        return Ok(None);
                    }
                    let start_time =
                        DateTime::parse_from_rfc3339(&entry.start_time)?.with_timezone(&Utc);
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
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Duration, Local, Utc};
    use log::debug;
    use std::path::Path;
    use tempfile::NamedTempFile;

    use crate::fitbit_client::FitbitClient;
    use garmin_lib::common::garmin_config::GarminConfig;

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
        let offset = client.get_client_offset().await?;
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
}
