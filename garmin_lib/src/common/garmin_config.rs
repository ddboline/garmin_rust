#![allow(clippy::wrong_self_convention)]
#![allow(clippy::cognitive_complexity)]

use anyhow::{format_err, Error};
use std::env::var;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

/// GarminConfig holds configuration information which can be set either through environment variables or the config.env file,
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

macro_rules! set_config_from_env {
    ($s:ident, $id:ident, $name:literal) => {
        if let Ok($id) = var($name) {
            $s.$id = $id.to_string()
        }
    };
}

impl GarminConfigInner {
    /// Some variables have natural default values, which we set in the new() method.
    pub fn new() -> GarminConfigInner {
        let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());

        let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
        let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);
        let default_summary_cache = format!("{}/.garmin_cache/run/summary_cache", home_dir);
        let default_fitbit_dir = format!("{}/.garmin_cache/run/fitbit_cache", home_dir);

        GarminConfigInner {
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
            summary_cache: default_summary_cache,
            port: 8000,
            n_db_workers: 2,
            secret_key: "0123".repeat(8),
            domain: "localhost".to_string(),
            fitbit_tokenfile: format!("{}/.fitbit_tokens", home_dir),
            strava_tokenfile: format!("{}/.stravacli", home_dir),
            fitbit_cachedir: default_fitbit_dir,
            home_dir,
            ..Default::default()
        }
    }

    /// Each variable maps to an environment variable, if the variable exists, use it.
    pub fn from_env(mut self) -> GarminConfigInner {
        set_config_from_env!(self, home_dir, "HOME");
        set_config_from_env!(self, pgurl, "PGURL");
        set_config_from_env!(self, maps_api_key, "MAPS_API_KEY");
        set_config_from_env!(self, gps_bucket, "GPS_BUCKET");
        set_config_from_env!(self, cache_bucket, "CACHE_BUCKET");
        set_config_from_env!(self, gps_dir, "GPS_DIR");
        set_config_from_env!(self, cache_dir, "CACHE_DIR");
        if let Ok(port) = var("PORT") {
            self.port = port.parse().unwrap_or(8000)
        }
        set_config_from_env!(self, summary_cache, "SUMMARY_CACHE");
        set_config_from_env!(self, summary_bucket, "SUMMARY_BUCKET");
        if let Ok(n_db_workers_str) = var("N_DB_WORKERS") {
            if let Ok(n_db_workers) = n_db_workers_str.parse() {
                self.n_db_workers = n_db_workers
            }
        }
        set_config_from_env!(self, secret_key, "SECRET_KEY");
        set_config_from_env!(self, domain, "DOMAIN");
        set_config_from_env!(self, google_secret_file, "GOOGLE_SECRET_FILE");
        set_config_from_env!(self, google_token_path, "GOOGLE_TOKEN_PATH");
        set_config_from_env!(self, telegram_bot_token, "TELEGRAM_BOT_TOKEN");
        set_config_from_env!(self, fitbit_clientid, "FITBIT_CLIENTID");
        set_config_from_env!(self, fitbit_clientsecret, "FITBIT_CLIENTSECRET");
        set_config_from_env!(self, fitbit_tokenfile, "FITBIT_TOKENFILE");
        set_config_from_env!(self, fitbit_cachedir, "FITBIT_CACHEDIR");
        set_config_from_env!(self, fitbit_bucket, "FITBIT_BUCKET");
        set_config_from_env!(self, strava_tokenfile, "STRAVA_TOKENFILE");
        set_config_from_env!(self, garmin_connect_email, "GARMIN_CONNECT_EMAIL");
        set_config_from_env!(self, garmin_connect_password, "GARMIN_CONNECT_PASSWORD");
        self
    }
}

impl GarminConfig {
    pub fn new() -> GarminConfig {
        GarminConfig(Arc::new(GarminConfigInner::new()))
    }

    /// Pull configuration from a file if it exists, first look for a config.env file in the current directory,
    /// then try ${HOME}/.config/garmin_rust/config.env,
    /// if that doesn't exist fall back on the default behaviour of dotenv
    /// Panic if required variables aren't set appropriately.
    pub fn get_config(fname: Option<&str>) -> Result<GarminConfig, Error> {
        let home_dir = var("HOME").map_err(|_| format_err!("No HOME directory..."))?;

        let default_fname = format!("{}/.config/garmin_rust/config.env", home_dir);

        let env_file = match fname {
            Some(fname) if Path::new(fname).exists() => fname.to_string(),
            _ => default_fname,
        };

        dotenv::dotenv().ok();

        if Path::new(&env_file).exists() {
            dotenv::from_path(&env_file).ok();
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
            Ok(GarminConfig(Arc::new(conf)))
        }
    }
}

impl Deref for GarminConfig {
    type Target = GarminConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
