use convert_case::{Case, Casing};
use log::debug;
use maplit::hashmap;
use reqwest::{header::HeaderMap, Client, Response};
use reqwest_oauth1::{OAuthClientProvider, Secrets};
use select::{document::Document, predicate::Name};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};
use time::{macros::format_description, Date, Duration as TimeDuration, OffsetDateTime, UtcOffset};
use time_tz::OffsetDateTimeExt;
use tokio::{fs, fs::File, io::AsyncWriteExt};
use tokio_stream::StreamExt;
use url::{form_urlencoded, Url};

use fitbit_lib::{
    scale_measurement::{ScaleMeasurement, GRAMS_PER_POUND},
    GarminConnectHrData,
};
use garmin_lib::{
    date_time_wrapper::DateTimeWrapper, errors::GarminError as Error, garmin_config::GarminConfig,
};
use garmin_models::garmin_connect_activity::{GarminConnectActivity, GarminConnectSocialProfile};

const HTTP_USER_AGENT: &str = "GCM-iOS-5.7.2.1";
const SSO_USER_AGENT: &str = "com.garmin.android.apps.connectmobile";
const SSO_URLBASE: &str = "https://sso.garmin.com";
const API_URLBASE: &str = "https://connectapi.garmin.com";

#[derive(Deserialize, Debug)]
pub struct GarminConnectWeightEntry {
    #[serde(rename = "samplePk")]
    pub sample_primary_key: i64,
    #[serde(rename = "calendarDate")]
    pub calendar_date: Date,
    pub date: u64,
    #[serde(rename = "timestampGMT")]
    pub timestamp_gmt: u64,
    pub weight: f64,
}

impl GarminConnectWeightEntry {
    #[must_use]
    pub fn weight_in_lbs(&self) -> f64 {
        self.weight / GRAMS_PER_POUND
    }
}

#[derive(Deserialize, Debug)]
pub struct DailyWeightView {
    #[serde(rename = "startDate")]
    pub start_date: Date,
    #[serde(rename = "endDate")]
    pub end_date: Date,
    #[serde(rename = "dateWeightList")]
    pub date_weight_list: Vec<GarminConnectWeightEntry>,
}

#[derive(Deserialize, Debug)]
pub struct DailyWeightSummaries {
    #[serde(rename = "summaryDate")]
    pub summary_date: Date,
    #[serde(rename = "latestWeight")]
    pub latest_weight: GarminConnectWeightEntry,
}

#[derive(Deserialize, Debug)]
pub struct GarminConnectWeightSummaries {
    #[serde(rename = "dailyWeightSummaries")]
    pub daily_weight_summaries: Vec<DailyWeightSummaries>,
}

#[derive(Serialize, Debug)]
struct GarminConnectWeightPayload {
    #[serde(rename = "dateTimestamp")]
    datetimestamp: StackString,
    #[serde(rename = "gmtTimestamp")]
    gmt_timestamp: StackString,
    #[serde(rename = "unitKey")]
    unit_key: StackString,
    value: f64,
}

impl TryFrom<&ScaleMeasurement> for GarminConnectWeightPayload {
    type Error = Error;

    fn try_from(value: &ScaleMeasurement) -> Result<Self, Self::Error> {
        let local = DateTimeWrapper::local_tz();

        let datetimestamp = value
            .datetime
            .to_timezone(local)
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].00"
            ))?
            .into();
        let gmt_timestamp = value
            .datetime
            .to_offset(UtcOffset::UTC)
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].00"
            ))?
            .into();
        let value = value.mass;
        Ok(Self {
            datetimestamp,
            gmt_timestamp,
            unit_key: "lbs".into(),
            value,
        })
    }
}

impl TryFrom<ScaleMeasurement> for GarminConnectWeightPayload {
    type Error = Error;

    fn try_from(value: ScaleMeasurement) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd)]
struct OAuth1Token {
    oauth_token: StackString,
    oauth_token_secret: StackString,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd)]
struct OAuth2Token {
    token: OAuth2TokenInner,
    expires_at: i64,
    refresh_token_expires_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd)]
struct OAuth2TokenInner {
    scope: StackString,
    jti: StackString,
    token_type: StackString,
    access_token: StackString,
    refresh_token: StackString,
    expires_in: i64,
    refresh_token_expires_in: i64,
}

impl From<OAuth2TokenInner> for OAuth2Token {
    fn from(token: OAuth2TokenInner) -> Self {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let expires_at = now + token.expires_in;
        let refresh_token_expires_at = now + token.refresh_token_expires_in;

        Self {
            token,
            expires_at,
            refresh_token_expires_at,
        }
    }
}

impl OAuth2Token {
    pub fn expired(&self) -> bool {
        self.expires_at < OffsetDateTime::now_utc().unix_timestamp()
    }

    pub fn auth_header(&self) -> StackString {
        let token_type = self.token.token_type.as_str().to_case(Case::Title);
        let access_token = &self.token.access_token;
        format_sstr!("{token_type} {access_token}")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Tokens {
    oauth1_token: OAuth1Token,
    oauth2_token: OAuth2Token,
}

#[derive(Default, Debug)]
pub struct GarminConnectClient {
    pub config: GarminConfig,
    pub client: Client,
    consumer_key: StackString,
    consumer_secret: StackString,
    oauth1_token: Option<OAuth1Token>,
    oauth2_token: Option<OAuth2Token>,
}

impl GarminConnectClient {
    /// # Errors
    /// Returns error if client configuration fails or oauth consumer key is not
    /// present
    pub fn new(config: GarminConfig) -> Result<Self, Error> {
        let client = Client::builder().cookie_store(true).build()?;
        let consumer_key = config
            .garmin_connect_oauth_consumer_key
            .clone()
            .ok_or_else(|| Error::StaticCustomError("No consumer key"))?;
        let consumer_secret = config
            .garmin_connect_oauth_consumer_secret
            .clone()
            .ok_or_else(|| Error::StaticCustomError("No consumer secret"))?;

        Ok(Self {
            config,
            client,
            consumer_key,
            consumer_secret,
            ..Self::default()
        })
    }

    /// # Errors
    /// Returns error in login fails or oauth2 token not found or refersh token
    /// fails
    pub async fn init(&mut self) -> Result<GarminConnectSocialProfile, Error> {
        if self.load().await.is_err() {
            let profile = self.login().await?;
            self.dump().await?;
            Ok(profile)
        } else {
            let oauth2_token = self
                .oauth2_token
                .as_ref()
                .ok_or_else(|| Error::StaticCustomError("No Oauth2 Token"))?;
            if oauth2_token.expired() {
                self.refresh_oauth2().await?;
                self.dump().await?;
            }
            self.login().await
        }
    }

    /// # Errors
    /// Returns error if login fails
    pub async fn login(&mut self) -> Result<GarminConnectSocialProfile, Error> {
        let referer = self.init_cookies().await?;

        let sso_embed = format_sstr!("{SSO_URLBASE}/sso/embed");
        let signin_params = hashmap! {
            "id" => "gauth-widget",
            "embedWidged" => "true",
            "gauthHost" => sso_embed.as_str(),
            "service" => sso_embed.as_str(),
            "source" => sso_embed.as_str(),
            "redirectAfterAccountLoginUrl" => sso_embed.as_str(),
            "redirectAfterAccountCreationUrl" => sso_embed.as_str(),
        };
        let mut url = Url::parse(&format_sstr!("{SSO_URLBASE}/sso/signin"))?;
        for (k, v) in &signin_params {
            url.query_pairs_mut().append_pair(k, v);
        }
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HTTP_USER_AGENT.parse()?);
        headers.insert("referer", referer.parse()?);
        let referer = url.to_string();

        let buf = self
            .client
            .get(url.clone())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let csrf_token = Self::extract_csrf(&buf)
            .ok_or_else(|| Error::StaticCustomError("Failed to extract csrf"))?;

        debug!("csrf_token {csrf_token}");

        let data = hashmap! {
            "username" => self.config.garmin_connect_email.as_str(),
            "password" => self.config.garmin_connect_password.as_str(),
            "embed" => "true",
            "_csrf" => csrf_token.as_str(),
        };

        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HTTP_USER_AGENT.parse()?);
        headers.insert("referer", referer.parse()?);

        let text = self
            .client
            .post(url)
            .headers(headers)
            .form(&data)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let title = Self::get_title(&text);
        if title != Some("Success".into()) {
            return Err(Error::StaticCustomError("Login failed"));
        }

        let ticket =
            Self::get_ticket(&text).ok_or_else(|| Error::StaticCustomError("Ticket not found"))?;
        let oauth1_token = self.get_oauth1_token(&ticket).await?;
        let oauth2_token = self.exchange(&oauth1_token).await?;

        self.oauth1_token.replace(oauth1_token);
        self.oauth2_token.replace(oauth2_token);

        self.get_user_profile().await
    }

    /// # Errors
    /// Returns error if oauth1/oauth2 tokens are not found or if saving to file
    /// fails.
    pub async fn dump(&self) -> Result<(), Error> {
        let oauth1_token = self
            .oauth1_token
            .clone()
            .ok_or_else(|| Error::StaticCustomError("No Oauth1 Token"))?;
        let oauth2_token = self
            .oauth2_token
            .clone()
            .ok_or_else(|| Error::StaticCustomError("No Oauth2 Token"))?;
        let tokens = Tokens {
            oauth1_token,
            oauth2_token,
        };

        let mut f = File::create(&self.config.garmin_connect_tokenfile).await?;
        let token_js = serde_json::to_vec(&tokens)?;
        f.write_all(&token_js).await?;
        Ok(())
    }

    /// # Errors
    /// Returns error if loading file or deserializing token fails
    pub async fn load(&mut self) -> Result<(), Error> {
        let buf = fs::read(&self.config.garmin_connect_tokenfile).await?;
        let tokens: Tokens = serde_json::from_slice(&buf)?;
        self.oauth1_token.replace(tokens.oauth1_token);
        self.oauth2_token.replace(tokens.oauth2_token);
        Ok(())
    }

    /// # Errors
    /// Returns error if missing oauth1 token or exchange fails
    pub async fn refresh_oauth2(&mut self) -> Result<(), Error> {
        let oauth1_token = self
            .oauth1_token
            .as_ref()
            .ok_or_else(|| Error::StaticCustomError("No Oauth1 Token"))?;
        self.oauth2_token
            .replace(self.exchange(oauth1_token).await?);
        Ok(())
    }

    #[must_use]
    pub fn get_title(buf: &str) -> Option<StackString> {
        Document::from(buf)
            .find(Name("title"))
            .find_map(|node| node.children().find_map(|n| n.as_text().map(Into::into)))
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn get_user_profile(&self) -> Result<GarminConnectSocialProfile, Error> {
        self.api_json("/userprofile-service/socialProfile").await
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn get_activities(
        &self,
        start: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<GarminConnectActivity>, Error> {
        let start = start.unwrap_or(0);
        let limit = limit.unwrap_or(20);
        let path = format_sstr!(
            "/activitylist-service/activities/search/activities?start={start}&limit={limit}"
        );
        self.api_json(&path).await
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn get_heartrate(&self, date: Date) -> Result<GarminConnectHrData, Error> {
        let path = format_sstr!("/wellness-service/wellness/dailyHeartRate?date={date}");
        self.api_json(&path).await
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn download_activity(&self, activity_id: i64) -> Result<PathBuf, Error> {
        let path = format_sstr!("/download-service/files/activity/{activity_id}");
        let output = self
            .config
            .download_directory
            .join(format_sstr!("{activity_id}.zip"));

        let total_bytes = self.api_download(&path, &output).await?;

        if total_bytes == 0 || !output.exists() {
            return Err(Error::StaticCustomError("Download failed"));
        }

        Ok(output)
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn upload_weight(&self, measurement: &ScaleMeasurement) -> Result<(), Error> {
        let payload: GarminConnectWeightPayload = measurement.try_into()?;
        println!("{}", serde_json::to_string(&payload)?);

        self.api_post("/weight-service/user-weight", &payload).await
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn get_weight(&self, date: Date) -> Result<DailyWeightView, Error> {
        let path = format_sstr!("/weight-service/weight/dayview/{date}");
        self.api_json(&path).await
    }

    /// # Errors
    /// Returns error if api call fails or deserialization fails
    pub async fn get_weights(
        &self,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<GarminConnectWeightSummaries, Error> {
        let today = OffsetDateTime::now_utc().date();
        let start_date = start_date.unwrap_or_else(|| today - TimeDuration::days(7));
        let end_date = end_date.unwrap_or(today);
        let path =
            format_sstr!("/weight-service/weight/range/{start_date}/{end_date}?includeAll=true");
        self.api_json(&path).await
    }

    fn get_api_headers(&self) -> Result<HeaderMap, Error> {
        let oauth2_token = self
            .oauth2_token
            .as_ref()
            .ok_or_else(|| Error::StaticCustomError("No Oauth2 Token"))?;
        if oauth2_token.expired() {
            return Err(Error::StaticCustomError("Oauth2 Token Expired"));
        }
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", oauth2_token.auth_header().parse()?);
        headers.insert("User-Agent", HTTP_USER_AGENT.parse()?);
        Ok(headers)
    }

    async fn api_request(&self, path: &str) -> Result<Response, Error> {
        let url: Url = format_sstr!("{API_URLBASE}{path}").parse()?;
        let headers = self.get_api_headers()?;

        self.client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()
            .map_err(Into::into)
    }

    async fn api_post<T: Serialize>(&self, path: &str, payload: &T) -> Result<(), Error> {
        let url: Url = format_sstr!("{API_URLBASE}{path}").parse()?;
        let headers = self.get_api_headers()?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(payload)
            .send()
            .await?
            .error_for_status()?;
        if response.status().as_u16() != 204 {
            return Err(Error::CustomError(format_sstr!(
                "Unexpected response {}",
                response.status().as_str()
            )));
        }
        Ok(())
    }

    async fn api_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let response = self.api_request(path).await?;
        response.json().await.map_err(Into::into)
    }

    async fn api_download(&self, path: &str, output: &Path) -> Result<usize, Error> {
        let mut f = File::create(output).await?;
        let response = self.api_request(path).await?;

        let mut total_bytes = 0;
        let mut stream = response.bytes_stream();
        while let Some(item) = stream.next().await {
            let buf = item?;
            total_bytes += buf.len();
            f.write_all(&buf).await?;
        }
        Ok(total_bytes)
    }

    async fn init_cookies(&self) -> Result<StackString, Error> {
        let sso = format_sstr!("{SSO_URLBASE}/sso");
        let mut url = Url::parse(&format_sstr!("{sso}/embed"))?;
        url.query_pairs_mut()
            .append_pair("id", "gauth-widget")
            .append_pair("embedWidged", "true")
            .append_pair("gauthHost", sso.as_str());
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HTTP_USER_AGENT.parse()?);
        let response = self.client.get(url).headers(headers).send().await?;
        let referer = response.url().to_string();
        Ok(referer.into())
    }

    async fn exchange(&self, oauth1_token: &OAuth1Token) -> Result<OAuth2Token, Error> {
        let secrets = self.get_secrets().token(
            oauth1_token.oauth_token.as_str(),
            oauth1_token.oauth_token_secret.as_str(),
        );
        let base_url = format_sstr!("{API_URLBASE}/oauth-service/oauth/");
        let url: Url = format_sstr!("{base_url}exchange/user/2.0").parse()?;
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", SSO_USER_AGENT.parse()?);
        headers.insert("Content-Type", "application/x-www-form-urlencoded".parse()?);

        let client = self.client.clone();

        let token: OAuth2TokenInner = client
            .oauth1(secrets)
            .post(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        debug!("{token:?}");
        Ok(token.into())
    }

    fn extract_csrf(buf: &str) -> Option<StackString> {
        Document::from(buf).find(Name("input")).find_map(|node| {
            node.attr("name").and_then(|name| {
                if name == "_csrf" {
                    node.attr("value").map(Into::into)
                } else {
                    None
                }
            })
        })
    }

    fn get_ticket(buf: &str) -> Option<StackString> {
        let prefix = "embed?ticket=";
        let offset = prefix.len();
        let start = buf.find(prefix)?;
        let end = buf[start..].find('"')?;
        let ticket = &buf[start + offset..start + end];
        Some(ticket.into())
    }

    fn get_secrets(&self) -> Secrets {
        Secrets::new(self.consumer_key.as_str(), self.consumer_secret.as_str())
    }

    async fn get_oauth1_token(&self, ticket: &str) -> Result<OAuth1Token, Error> {
        let base_url = format_sstr!("{API_URLBASE}/oauth-service/oauth/");
        let login_url = "https://sso.garmin.com/sso/embed";
        let mut url: Url = format_sstr!("{base_url}preauthorized").parse()?;
        url.query_pairs_mut()
            .append_pair("ticket", ticket)
            .append_pair("login-url", login_url)
            .append_pair("accepts-mfa-tokens", "true");

        let secrets = self.get_secrets();

        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", SSO_USER_AGENT.parse()?);
        let client = self.client.clone();
        let text = client
            .oauth1(secrets)
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        debug!("{text}");

        let mut oauth_token: Option<StackString> = None;
        let mut oauth_token_secret: Option<StackString> = None;
        for (k, v) in form_urlencoded::parse(text.as_bytes()) {
            if k == "oauth_token" {
                oauth_token.replace(v.into());
            } else if k == "oauth_token_secret" {
                oauth_token_secret.replace(v.into());
            }
        }
        let oauth_token = oauth_token.ok_or_else(|| Error::StaticCustomError("no oauth token"))?;
        let oauth_token_secret =
            oauth_token_secret.ok_or_else(|| Error::StaticCustomError("no oauth token secret"))?;

        Ok(OAuth1Token {
            oauth_token,
            oauth_token_secret,
        })
    }
}

#[cfg(test)]
mod tests {
    // use fitbit_lib::scale_measurement::ScaleMeasurement;
    // use std::{collections::HashMap, convert::TryInto};
    // use time::{Duration, OffsetDateTime, UtcOffset};
    // use tokio::fs::remove_file;

    use garmin_lib::errors::GarminError as Error;
    // use garmin_lib::garmin_config::GarminConfig;
    // use garmin_utils::pgpool::PgPool;

    use crate::garmin_connect_client::GarminConnectClient;
    // use crate::garmin_connect_client::{
    //     GarminConnectWeightPayload, GRAMS_PER_POUND,
    // };

    // #[tokio::test]
    // #[ignore]
    // async fn test_garmin_connect_client() -> Result<(), Error> {
    //     let config = GarminConfig::get_config(None)?;
    //     let mut client = GarminConnectClient::new(config)?;

    //     let profile = client.login().await?;
    //     assert_eq!(profile.username, "ddboline");

    //     assert!(client.oauth1_token.is_some());
    //     assert!(client.oauth2_token.is_some());
    //     client.dump().await?;

    //     let oauth1_token = client.oauth1_token.take().unwrap();
    //     let oauth2_token = client.oauth2_token.take().unwrap();

    //     client.load().await?;

    //     assert!(client.oauth1_token.is_some());
    //     assert!(client.oauth2_token.is_some());

    //     assert_eq!(client.oauth1_token.as_ref().unwrap(), &oauth1_token);
    //     assert_eq!(client.oauth2_token.as_ref().unwrap(), &oauth2_token);

    //     let oauth2_token = client.oauth2_token.as_ref().unwrap();

    //     if oauth2_token.expired() {
    //         client.refresh_oauth2().await?;
    //     }
    //     let activities = client.get_activities(Some(0), Some(5)).await?;
    //     assert_eq!(activities.len(), 5);

    //     let date = (OffsetDateTime::now_utc() - Duration::days(1)).date();

    //     let heartrates = client.get_heartrate(date).await?;

    //     assert!(heartrates.heartrate_values.is_some());

    //     let values = heartrates.heartrate_values.unwrap();

    //     assert!(values.len() > 0);

    //     let output = client.download_activity(18201068560).await?;

    //     assert!(output.exists());

    //     remove_file(output).await?;
    //     Ok(())
    // }

    // #[tokio::test]
    // #[ignore]
    // async fn test_weight() -> Result<(), Error> {
    //     let config = GarminConfig::get_config(None)?;
    //     let pool = PgPool::new(&config.pgurl)?;

    //     let mut client = GarminConnectClient::new(config)?;
    //     let profile = client.init().await?;
    //     assert_eq!(profile.username, "ddboline");

    //     let weights = client.get_weights(None, None).await?;
    //     let mut start_date = None;
    //     let mut end_date = None;
    //     for dws in &weights.daily_weight_summaries {
    //         let d = dws.latest_weight.calendar_date;
    //         if start_date.is_none() {
    //             start_date.replace(d);
    //         }
    //         if end_date.is_none() {
    //             end_date.replace(d);
    //         }
    //         start_date = start_date.min(Some(d));
    //         end_date = end_date.max(Some(d));
    //     }
    //     let start_date = start_date.unwrap();
    //     let end_date = end_date.unwrap();
    //     println!("{start_date} {end_date}");

    //     let measurements =
    //         ScaleMeasurement::read_from_db(&pool, Some(start_date), Some(end_date), None, None)
    //             .await?;
    //     let mut measurement_map: HashMap<_, _> = measurements
    //         .into_iter()
    //         .map(|m| (m.datetime.to_offset(UtcOffset::UTC).date(), m))
    //         .collect();

    //     for dws in &weights.daily_weight_summaries {
    //         let d = dws.latest_weight.calendar_date;

    //         if let Some(measurement) = measurement_map.get_mut(&d) {
    //             if (dws.latest_weight.weight - (measurement.mass * GRAMS_PER_POUND)).abs() < 1.0
    //                 && measurement.connect_primary_key.is_none()
    //             {
    //                 measurement
    //                     .set_connect_primary_key(dws.latest_weight.sample_primary_key, &pool)
    //                     .await?;
    //             }
    //         }
    //     }

    //     let mut weights = client.get_weights(None, None).await?;
    //     assert!(weights.daily_weight_summaries.len() > 0);
    //     let weight = weights.daily_weight_summaries.pop().unwrap();
    //     let date = weight.latest_weight.calendar_date;

    //     let mut weight_view = client.get_weight(date).await?;
    //     assert_eq!(weight_view.date_weight_list.len(), 1);
    //     let new_weight = weight_view.date_weight_list.pop().unwrap();
    //     assert_eq!(
    //         weight.latest_weight.sample_primary_key,
    //         new_weight.sample_primary_key
    //     );
    //     assert_eq!(weight.latest_weight.weight, new_weight.weight);
    //     println!("{new_weight:?}");
    //     Ok(())
    // }

    #[test]
    fn test_get_csrf() -> Result<(), Error> {
        let text = include_str!("../../tests/data/garmin_connect_signin.html");

        let csrf = GarminConnectClient::extract_csrf(text).unwrap();

        assert_eq!(csrf, "06E7CB7A16537E772CAA1C96AC81B65FE29B0BFE5B02E73BFF23F6C6649361518D8AE49C56D4C76D1F37DE5E50297E86D2FD");
        Ok(())
    }

    #[test]
    fn test_get_title() -> Result<(), Error> {
        let text = include_str!("../../tests/data/garmin_connect_title_page.html");

        let title = GarminConnectClient::get_title(&text).unwrap();

        assert_eq!(title, "Success");
        Ok(())
    }

    #[test]
    fn test_get_ticket() -> Result<(), Error> {
        let text = include_str!("../../tests/data/garmin_connect_title_page.html");

        let ticket = GarminConnectClient::get_ticket(&text).unwrap();

        assert_eq!(ticket, "ST-01661298-T7v2orXQYEtXD5G3Buvq-cas");
        Ok(())
    }

    // #[tokio::test]
    // #[ignore]
    // async fn test_post_weight() -> Result<(), Error> {
    //     let config = GarminConfig::get_config(None)?;
    //     let pool = PgPool::new(&config.pgurl)?;
    //     let mut measurements =
    //         ScaleMeasurement::read_from_db(&pool, None, None, Some(0), Some(1)).await?;
    //     assert_eq!(measurements.len(), 1);
    //     let measurement = measurements.pop().unwrap();
    //     let payload: GarminConnectWeightPayload = measurement.try_into()?;
    //     let text = serde_json::to_string(&payload)?;
    //     assert_eq!(
    //         text,
    //         r#"{"dateTimestamp":"2016-02-24T04:00:00.00","gmtTimestamp":"2016-02-24T09:00:00.00","unitKey":"lbs","value":174.8}"#
    //     );
    //     Ok(())
    // }
}
