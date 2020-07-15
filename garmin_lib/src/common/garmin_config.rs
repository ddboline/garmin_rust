#![allow(clippy::wrong_self_convention)]
#![allow(clippy::cognitive_complexity)]

use anyhow::{format_err, Error};
use serde::Deserialize;
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use stack_string::StackString;

/// `GarminConfig` holds configuration information which can be set either
/// through environment variables or the config.env file, see the dotenv crate
/// for more information about the config file format.
#[derive(Default, Debug, Deserialize)]
pub struct GarminConfigInner {
    #[serde(default = "default_home_dir")]
    pub home_dir: PathBuf,
    pub pgurl: StackString,
    pub maps_api_key: StackString,
    pub gps_bucket: StackString,
    pub cache_bucket: StackString,
    #[serde(default = "default_gps_dir")]
    pub gps_dir: PathBuf,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    #[serde(default = "default_port")]
    pub port: u32,
    #[serde(default = "default_summary_cache")]
    pub summary_cache: PathBuf,
    pub summary_bucket: StackString,
    #[serde(default = "default_n_db_workers")]
    pub n_db_workers: usize,
    #[serde(default = "default_secret_key")]
    pub secret_key: StackString,
    #[serde(default = "default_domain")]
    pub domain: StackString,
    pub google_secret_file: PathBuf,
    pub google_token_path: PathBuf,
    pub telegram_bot_token: Option<StackString>,
    pub fitbit_clientid: StackString,
    pub fitbit_clientsecret: StackString,
    #[serde(default = "default_fitbit_tokenfile")]
    pub fitbit_tokenfile: PathBuf,
    #[serde(default = "default_fitbit_cachedir")]
    pub fitbit_cachedir: PathBuf,
    pub fitbit_bucket: StackString,
    #[serde(default = "default_strava_tokenfile")]
    pub strava_tokenfile: PathBuf,
    pub garmin_connect_email: StackString,
    pub garmin_connect_password: StackString,
}

fn default_home_dir() -> PathBuf {
    dirs::home_dir().expect("No home directory")
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

#[derive(Default, Debug, Clone)]
pub struct GarminConfig(Arc<GarminConfigInner>);

fn cache_dir() -> PathBuf {
    default_home_dir().join(".garmin_cache").join("run")
}
fn default_gps_dir() -> PathBuf {
    cache_dir().join("gps_tracks")
}
fn default_cache_dir() -> PathBuf {
    cache_dir().join("cache")
}
fn default_summary_cache() -> PathBuf {
    cache_dir().join("summary_cache")
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

impl GarminConfigInner {
    /// Some variables have natural default values, which we set in the new()
    /// method.
    pub fn new() -> Self {
        Self {
            home_dir: default_home_dir(),
            gps_dir: default_gps_dir(),
            cache_dir: default_cache_dir(),
            port: default_port(),
            summary_cache: default_summary_cache(),
            n_db_workers: default_n_db_workers(),
            secret_key: default_secret_key(),
            domain: default_domain(),
            fitbit_tokenfile: default_fitbit_tokenfile(),
            fitbit_cachedir: default_fitbit_cachedir(),
            strava_tokenfile: default_strava_tokenfile(),
            ..Self::default()
        }
    }
}

impl GarminConfig {
    pub fn new() -> Self {
        Self(Arc::new(GarminConfigInner::new()))
    }

    /// Pull configuration from a file if it exists,
    /// first look for a config.env file in the current directory,
    /// then try `${HOME}/.config/garmin_rust/config.env`,
    /// if that doesn't exist fall back on the default behaviour of dotenv
    /// Panic if required variables aren't set appropriately.
    pub fn get_config(fname: Option<&str>) -> Result<Self, Error> {
        let config_dir = dirs::config_dir().ok_or_else(|| format_err!("No CONFIG directory"))?;
        let default_fname = config_dir.join("garmin_rust").join("config.env");

        let env_file = match fname.map(|x| Path::new(x)) {
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
        } else if &conf.summary_bucket == "" {
            Err(format_err!("No SUMMARY_BUCKET specified"))
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
