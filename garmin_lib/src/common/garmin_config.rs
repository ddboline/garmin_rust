#![allow(clippy::wrong_self_convention)]
#![allow(clippy::cognitive_complexity)]

use anyhow::{format_err, Error};
use derive_more::{Deref, Into};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    convert::{TryFrom, TryInto},
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

use super::strava_timezone::StravaTz;

/// `GarminConfig` holds configuration information which can be set either
/// through environment variables or the config.env file, see the dotenv crate
/// for more information about the config file format.
#[derive(Debug, Deserialize)]
pub struct GarminConfigInner {
    #[serde(default = "default_home_dir")]
    pub home_dir: PathBuf,
    pub pgurl: StackString,
    #[serde(default = "default_secret_key")]
    pub maps_api_key: StackString,
    #[serde(default = "default_gps_bucket")]
    pub gps_bucket: StackString,
    #[serde(default = "default_gps_bucket")]
    pub cache_bucket: StackString,
    #[serde(default = "default_gps_dir")]
    pub gps_dir: PathBuf,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    #[serde(default = "default_host")]
    pub host: StackString,
    #[serde(default = "default_port")]
    pub port: u32,
    #[serde(default = "default_n_db_workers")]
    pub n_db_workers: usize,
    #[serde(default = "default_secret_key")]
    pub secret_key: StackString,
    #[serde(default = "default_domain")]
    pub domain: StackString,
    pub telegram_bot_token: Option<StackString>,
    #[serde(default = "default_gps_bucket")]
    pub fitbit_clientid: StackString,
    #[serde(default = "default_gps_bucket")]
    pub fitbit_clientsecret: StackString,
    #[serde(default = "default_fitbit_tokenfile")]
    pub fitbit_tokenfile: PathBuf,
    #[serde(default = "default_fitbit_cachedir")]
    pub fitbit_cachedir: PathBuf,
    #[serde(default = "default_gps_bucket")]
    pub fitbit_bucket: StackString,
    #[serde(default = "default_fitbit_endpoint")]
    pub fitbit_endpoint: Option<UrlWrapper>,
    #[serde(default = "default_fitbit_api_endpoint")]
    pub fitbit_api_endpoint: Option<UrlWrapper>,
    #[serde(default = "default_strava_tokenfile")]
    pub strava_tokenfile: PathBuf,
    pub strava_email: Option<StackString>,
    pub strava_password: Option<StackString>,
    #[serde(default = "default_strava_endpoint")]
    pub strava_endpoint: Option<UrlWrapper>,
    #[serde(default = "default_gps_bucket")]
    pub garmin_connect_email: StackString,
    #[serde(default = "default_gps_bucket")]
    pub garmin_connect_password: StackString,
    #[serde(default = "default_connect_sso_endpoint")]
    pub garmin_connect_sso_endpoint: Option<UrlWrapper>,
    #[serde(default = "default_connect_api_endpoint")]
    pub garmin_connect_api_endpoint: Option<UrlWrapper>,
    #[serde(default = "default_webdriver_path")]
    pub webdriver_path: PathBuf,
    #[serde(default = "default_webdriver_port")]
    pub webdriver_port: u32,
    #[serde(default = "default_chrome_path")]
    pub chrome_path: PathBuf,
    #[serde(default = "default_secret_path")]
    pub secret_path: PathBuf,
    #[serde(default = "default_secret_path")]
    pub jwt_secret_path: PathBuf,
    #[serde(default = "default_download_directory")]
    pub download_directory: PathBuf,
    pub default_time_zone: Option<StravaTz>,
}

fn default_home_dir() -> PathBuf {
    dirs::home_dir().expect("No home directory")
}
fn default_host() -> StackString {
    "0.0.0.0".into()
}
fn default_port() -> u32 {
    8000
}
fn default_secret_key() -> StackString {
    "0123".repeat(8).into()
}
fn default_domain() -> StackString {
    "localhost".into()
}
fn default_n_db_workers() -> usize {
    2
}
fn default_secret_path() -> PathBuf {
    dirs::config_dir()
        .unwrap()
        .join("aws_app_rust")
        .join("secret.bin")
}
fn cache_dir() -> PathBuf {
    default_home_dir().join(".garmin_cache").join("run")
}
fn default_gps_dir() -> PathBuf {
    cache_dir().join("gps_tracks")
}
fn default_cache_dir() -> PathBuf {
    cache_dir().join("cache")
}
fn default_fitbit_cachedir() -> PathBuf {
    cache_dir().join("fitbit_cache")
}
fn default_fitbit_tokenfile() -> PathBuf {
    default_home_dir().join(".fitbit_tokens")
}
fn default_strava_tokenfile() -> PathBuf {
    default_home_dir().join(".stravacli")
}
fn default_webdriver_path() -> PathBuf {
    default_home_dir().join("bin").join("chromedriver")
}
fn default_chrome_path() -> PathBuf {
    Path::new("/usr/bin/google-chrome").to_path_buf()
}
fn default_webdriver_port() -> u32 {
    4444
}
fn default_gps_bucket() -> StackString {
    "garmin_scripts_gps_files_ddboline".into()
}
fn default_download_directory() -> PathBuf {
    default_home_dir().join("Downloads")
}
fn default_fitbit_endpoint() -> Option<UrlWrapper> {
    "https://www.fitbit.com/".try_into().ok()
}
fn default_fitbit_api_endpoint() -> Option<UrlWrapper> {
    "https://api.fitbit.com/".try_into().ok()
}
fn default_strava_endpoint() -> Option<UrlWrapper> {
    "https://www.strava.com/".try_into().ok()
}
fn default_connect_sso_endpoint() -> Option<UrlWrapper> {
    "https://connect.garmin.com/signin".try_into().ok()
}
fn default_connect_api_endpoint() -> Option<UrlWrapper> {
    "https://connect.garmin.com/modern".try_into().ok()
}

impl Default for GarminConfigInner {
    fn default() -> Self {
        let default = r#"{"pgurl":""}"#;
        serde_json::from_str(default).unwrap()
    }
}

#[derive(Default, Debug, Clone)]
pub struct GarminConfig(Arc<GarminConfigInner>);

impl GarminConfig {
    /// Pull configuration from a file if it exists,
    /// first look for a config.env file in the current directory,
    /// then try `${HOME}/.config/garmin_rust/config.env`,
    /// if that doesn't exist fall back on the default behaviour of dotenv
    /// Panic if required variables aren't set appropriately.
    /// # Errors
    /// Returns error if init of config fails
    pub fn get_config(fname: Option<&str>) -> Result<Self, Error> {
        let config_dir = dirs::config_dir().ok_or_else(|| format_err!("No CONFIG directory"))?;
        let default_fname = config_dir.join("garmin_rust").join("config.env");

        let env_file = match fname.map(Path::new) {
            Some(fname) if fname.exists() => fname,
            _ => &default_fname,
        };

        dotenv::dotenv().ok();

        if env_file.exists() {
            dotenv::from_path(env_file).ok();
        } else if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        }

        let conf: GarminConfigInner = envy::from_env()?;

        if &conf.pgurl == "" {
            Err(format_err!("No PGURL specified"))
        } else if &conf.gps_bucket == "" {
            Err(format_err!("No GPS_BUCKET specified"))
        } else if &conf.cache_bucket == "" {
            Err(format_err!("No CACHE_BUCKET specified"))
        } else {
            Ok(Self(Arc::new(conf)))
        }
    }
}

impl Deref for GarminConfig {
    type Target = GarminConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Into, PartialEq, Deref, Eq)]
#[serde(into = "String", try_from = "String")]
pub struct UrlWrapper(Url);

impl From<UrlWrapper> for String {
    fn from(item: UrlWrapper) -> String {
        item.0.into()
    }
}

impl TryFrom<String> for UrlWrapper {
    type Error = Error;
    fn try_from(item: String) -> Result<Self, Self::Error> {
        Self::try_from(item.as_str())
    }
}

impl TryFrom<&str> for UrlWrapper {
    type Error = Error;
    fn try_from(item: &str) -> Result<Self, Self::Error> {
        let url: Url = item.parse()?;
        Ok(Self(url))
    }
}
