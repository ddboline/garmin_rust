#![allow(clippy::wrong_self_convention)]
extern crate dotenv;

use std::env::var;
use std::path::Path;

#[derive(Default, Debug)]
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
}

impl GarminConfig {
    pub fn new() -> GarminConfig {
        let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());

        let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
        let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);
        let default_summary_cache = format!("{}/.garmin_cache/run/summary_cache", home_dir);

        GarminConfig {
            pgurl: "".to_string(),
            maps_api_key: "".to_string(),
            gps_bucket: "".to_string(),
            cache_bucket: "".to_string(),
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
            port: 8000,
            summary_cache: default_summary_cache,
            summary_bucket: "".to_string(),
        }
    }

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
        self
    }

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
