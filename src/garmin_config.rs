#![allow(clippy::wrong_self_convention)]
use config::{Config, File};
use std::env::var;

#[derive(Default)]
pub struct GarminConfig {
    pub pgurl: Option<String>,
    pub maps_api_key: Option<String>,
    pub gps_bucket: Option<String>,
    pub cache_bucket: Option<String>,
    pub http_bucket: Option<String>,
    pub gps_dir: String,
    pub cache_dir: String,
    pub port: u32,
    pub summary_cache: Option<String>,
    pub summary_bucket: Option<String>,
}

impl GarminConfig {
    pub fn new() -> GarminConfig {
        let home_dir = var("HOME").unwrap_or("/tmp".to_string());

        let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
        let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);

        GarminConfig {
            pgurl: None,
            maps_api_key: None,
            gps_bucket: None,
            cache_bucket: None,
            http_bucket: None,
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
            port: 8000,
            summary_cache: None,
            summary_bucket: None,
        }
    }

    pub fn from_env(mut self) -> GarminConfig {
        if let Ok(pgurl) = var("PGURL") {
            self.pgurl = Some(pgurl.to_string())
        }
        if let Ok(maps_api_key) = var("MAPS_API_KEY") {
            self.maps_api_key = Some(maps_api_key.to_string())
        }
        if let Ok(gps_bucket) = var("GPS_BUCKET") {
            self.gps_bucket = Some(gps_bucket.to_string())
        }
        if let Ok(cache_bucket) = var("CACHE_BUCKET") {
            self.cache_bucket = Some(cache_bucket.to_string())
        }
        if let Ok(http_bucket) = var("HTTP_BUCKET") {
            self.http_bucket = Some(http_bucket.to_string())
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
            self.summary_cache = Some(summary_cache)
        }
        if let Ok(summary_bucket) = var("SUMMARY_BUCKET") {
            self.summary_bucket = Some(summary_bucket)
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
            self.pgurl = Some(pgurl.to_string())
        }
        if let Ok(maps_api_key) = settings.get_str("MAPS_API_KEY") {
            self.maps_api_key = Some(maps_api_key.to_string())
        }
        if let Ok(gps_bucket) = settings.get_str("GPS_BUCKET") {
            self.gps_bucket = Some(gps_bucket.to_string())
        }
        if let Ok(cache_bucket) = settings.get_str("CACHE_BUCKET") {
            self.cache_bucket = Some(cache_bucket.to_string())
        }
        if let Ok(http_bucket) = settings.get_str("HTTP_BUCKET") {
            self.http_bucket = Some(http_bucket.to_string())
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
            self.summary_cache = Some(summary_cache)
        }
        if let Ok(summary_bucket) = settings.get_str("SUMMARY_BUCKET") {
            self.summary_bucket = Some(summary_bucket)
        }
        self
    }
}
