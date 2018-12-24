#![allow(clippy::wrong_self_convention)]
use config::{Config, File};
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

    pub fn from_yml(mut self, filename: &str) -> GarminConfig {
        let settings = match Config::new().merge(File::with_name(filename)) {
            Ok(c) => c.clone(),
            Err(err) => {
                debug!("Failed to read yml file {}, {}", filename, err);
                Config::new()
            }
        };

        if let Ok(pgurl) = settings.get_str("PGURL") {
            self.pgurl = pgurl.to_string()
        }
        if let Ok(maps_api_key) = settings.get_str("MAPS_API_KEY") {
            self.maps_api_key = maps_api_key.to_string()
        }
        if let Ok(gps_bucket) = settings.get_str("GPS_BUCKET") {
            self.gps_bucket = gps_bucket.to_string()
        }
        if let Ok(cache_bucket) = settings.get_str("CACHE_BUCKET") {
            self.cache_bucket = cache_bucket.to_string()
        }
        if let Ok(gps_dir) = settings.get_str("GPS_DIR") {
            self.gps_dir = gps_dir.to_string()
        }
        if let Ok(cache_dir) = settings.get_str("CACHE_DIR") {
            self.cache_dir = cache_dir.to_string()
        }
        if let Ok(port) = settings.get_int("PORT") {
            self.port = port as u32
        }
        if let Ok(summary_cache) = settings.get_str("SUMMARY_CACHE") {
            self.summary_cache = summary_cache
        }
        if let Ok(summary_bucket) = settings.get_str("SUMMARY_BUCKET") {
            self.summary_bucket = summary_bucket
        }
        self
    }

    pub fn get_config() -> GarminConfig {
        let home_dir = var("HOME").expect("No HOME directory...");

        let conf_file = format!("{}/.config/garmin_rust/config.yml", home_dir);

        let conf = if Path::new(&conf_file).exists() {
            GarminConfig::new().from_yml(&conf_file)
        } else if Path::new("config.yml").exists() {
            GarminConfig::new().from_yml("config.yml")
        } else {
            GarminConfig::new()
        }
        .from_env();

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
