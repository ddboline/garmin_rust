#![allow(clippy::wrong_self_convention)]
#![allow(clippy::cognitive_complexity)]

use anyhow::{format_err, Error};
use std::{env::var, ops::Deref, path::Path, sync::Arc};

/// `GarminConfig` holds configuration information which can be set either through environment variables or the config.env file,
/// see the dotenv crate for more information about the config file format.
#[derive(Default, Debug)]
pub struct GarminConfigInner {
    pub home_dir: String,
    pub pgurl: String,
    pub maps_api_key: String,
    pub gps_bucket: String,
    pub cache_bucket: String,
    pub gps_dir: String,
    pub cache_dir: String,
    pub port: u32,
    pub summary_cache: String,
    pub summary_bucket: String,
    pub n_db_workers: usize,
    pub secret_key: String,
    pub domain: String,
    pub google_secret_file: String,
    pub google_token_path: String,
    pub telegram_bot_token: String,
    pub fitbit_clientid: String,
    pub fitbit_clientsecret: String,
    pub fitbit_tokenfile: String,
    pub fitbit_cachedir: String,
    pub fitbit_bucket: String,
    pub strava_tokenfile: String,
    pub garmin_connect_email: String,
    pub garmin_connect_password: String,
}

#[derive(Default, Debug, Clone)]
pub struct GarminConfig(Arc<GarminConfigInner>);

macro_rules! set_config_parse {
    ($s:ident, $id:ident) => {
        if let Some($id) = var(&stringify!($id).to_uppercase())
            .ok()
            .and_then(|x| x.parse().ok())
        {
            $s.$id = $id;
        }
    };
}

macro_rules! set_config_parse_default {
    ($s:ident, $id:ident, $d:expr) => {
        $s.$id = var(&stringify!($id).to_uppercase())
            .ok()
            .and_then(|x| x.parse().ok())
            .unwrap_or_else(|| $d);
    };
}

macro_rules! set_config_from_env {
    ($s:ident, $id:ident) => {
        if let Ok($id) = var(&stringify!($id).to_uppercase()) {
            $s.$id = $id.to_string()
        }
    };
}

impl GarminConfigInner {
    /// Some variables have natural default values, which we set in the new() method.
    pub fn new() -> Self {
        let home_dir = dirs::home_dir().unwrap_or_else(|| Path::new("/tmp").to_path_buf());
        let cache_dir = home_dir.join(".garmin_cache").join("run");

        let default_gps_dir = cache_dir.join("gps_tracks").to_string_lossy().into();
        let default_cache_dir = cache_dir.join("cache").to_string_lossy().into();
        let default_summary_cache = cache_dir.join("summary_cache").to_string_lossy().into();
        let default_fitbit_dir = cache_dir.join("fitbit_cache").to_string_lossy().into();
        let fitbit_tokenfile = home_dir.join(".fitbit_tokens").to_string_lossy().into();
        let strava_tokenfile = home_dir.join(".stravacli").to_string_lossy().into();

        Self {
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
            summary_cache: default_summary_cache,
            port: 8000,
            n_db_workers: 2,
            secret_key: "0123".repeat(8),
            domain: "localhost".to_string(),
            fitbit_tokenfile,
            strava_tokenfile,
            fitbit_cachedir: default_fitbit_dir,
            home_dir: home_dir.to_string_lossy().into(),
            ..Self::default()
        }
    }

    /// Each variable maps to an environment variable, if the variable exists, use it.
    pub fn from_env(mut self) -> Self {
        set_config_from_env!(self, pgurl);
        set_config_from_env!(self, maps_api_key);
        set_config_from_env!(self, gps_bucket);
        set_config_from_env!(self, cache_bucket);
        set_config_from_env!(self, gps_dir);
        set_config_from_env!(self, cache_dir);
        set_config_parse_default!(self, port, 8000);
        set_config_from_env!(self, summary_cache);
        set_config_from_env!(self, summary_bucket);
        set_config_parse!(self, n_db_workers);
        set_config_from_env!(self, secret_key);
        set_config_from_env!(self, domain);
        set_config_from_env!(self, google_secret_file);
        set_config_from_env!(self, google_token_path);
        set_config_from_env!(self, telegram_bot_token);
        set_config_from_env!(self, fitbit_clientid);
        set_config_from_env!(self, fitbit_clientsecret);
        set_config_from_env!(self, fitbit_tokenfile);
        set_config_from_env!(self, fitbit_cachedir);
        set_config_from_env!(self, fitbit_bucket);
        set_config_from_env!(self, strava_tokenfile);
        set_config_from_env!(self, garmin_connect_email);
        set_config_from_env!(self, garmin_connect_password);
        self
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

        let conf = GarminConfigInner::new().from_env();

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
