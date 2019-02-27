#![allow(clippy::wrong_self_convention)]

use std::env::var;
use std::path::Path;

/// GarminConfig holds configuration information which can be set either through environment variables or the config.env file,
/// see the dotenv crate for more information about the config file format.
#[derive(Default, Debug, Clone)]
pub struct GarminConfig {
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
}

impl GarminConfig {

    /// Some variables have natural default values, which we set in the new() method.
    pub fn new() -> GarminConfig {
        let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());

        let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
        let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);
        let default_summary_cache = format!("{}/.garmin_cache/run/summary_cache", home_dir);

        GarminConfig {
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
            summary_cache: default_summary_cache,
            port: 8000,
            n_db_workers: 2,
            secret_key: "0123".repeat(8),
            domain: "localhost".to_string(),
            ..Default::default()
        }
    }

    /// Each variable maps to an environment variable, if the variable exists, use it.
    pub fn from_env(mut self) -> GarminConfig {
        if let Ok(pgurl) = var("PGURL") {
            self.pgurl = pgurl.to_string()
        }
        if let Ok(maps_api_key) = var("MAPS_API_KEY") {
            self.maps_api_key = maps_api_key.to_string()
        }
        if let Ok(gps_bucket) = var("GPS_BUCKET") {
            self.gps_bucket = gps_bucket.to_string()
        }
        if let Ok(cache_bucket) = var("CACHE_BUCKET") {
            self.cache_bucket = cache_bucket.to_string()
        }
        if let Ok(gps_dir) = var("GPS_DIR") {
            self.gps_dir = gps_dir.to_string()
        }
        if let Ok(cache_dir) = var("CACHE_DIR") {
            self.cache_dir = cache_dir.to_string()
        }
        if let Ok(port) = var("PORT") {
            self.port = port.parse().unwrap_or(8000)
        }
        if let Ok(summary_cache) = var("SUMMARY_CACHE") {
            self.summary_cache = summary_cache
        }
        if let Ok(summary_bucket) = var("SUMMARY_BUCKET") {
            self.summary_bucket = summary_bucket
        }
        if let Ok(n_db_workers_str) = var("N_DB_WORKERS") {
            if let Ok(n_db_workers) = n_db_workers_str.parse() {
                self.n_db_workers = n_db_workers
            }
        }
        if let Ok(secret_key) = var("SECRET_KEY") {
            self.secret_key = secret_key
        }
        if let Ok(domain) = var("DOMAIN") {
            self.domain = domain
        }
        self
    }

    /// Pull configuration from a file if it exists, first look for a config.env file in the current directory,
    /// then try ${HOME}/.config/garmin_rust/config.env,
    /// if that doesn't exist fall back on the default behaviour of dotenv
    /// Panic if required variables aren't set appropriately.
    pub fn get_config(fname: Option<&str>) -> GarminConfig {
        let home_dir = var("HOME").expect("No HOME directory...");

        let default_fname = format!("{}/.config/garmin_rust/config.env", home_dir);

        let env_file = match fname {
            Some(fname) if Path::new(fname).exists() => fname.to_string(),
            _ => default_fname,
        };

        if Path::new(&env_file).exists() {
            dotenv::from_path(&env_file).ok();
        } else if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else {
            dotenv::dotenv().ok();
        }

        let conf = GarminConfig::new().from_env();

        if &conf.pgurl == "" {
            panic!("No PGURL specified")
        } else if &conf.gps_bucket == "" {
            panic!("No GPS_BUCKET specified")
        } else if &conf.cache_bucket == "" {
            panic!("No CACHE_BUCKET specified")
        } else if &conf.summary_bucket == "" {
            panic!("No SUMMARY_BUCKET specified")
        } else {
            conf
        }
    }
}
