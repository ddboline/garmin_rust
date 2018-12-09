pub struct GarminConfig {
    pub pgurl: String,
    pub maps_api_key: String,
    pub gps_bucket: String,
    pub cache_bucket: String,
    pub http_bucket: String,
    pub gps_dir: String,
    pub cache_dir: String,
}

impl GarminConfig {
    pub fn new() -> GarminConfig {
        let home_dir = env!("HOME");

        let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
        let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);

        GarminConfig {
            pgurl: "".to_string(),
            maps_api_key: "".to_string(),
            gps_bucket: "".to_string(),
            cache_bucket: "".to_string(),
            http_bucket: "".to_string(),
            gps_dir: default_gps_dir,
            cache_dir: default_cache_dir,
        }
    }

    pub fn from_env(mut self) -> GarminConfig {
        if let Some(pgurl) = option_env!("PGURL") {
            self.pgurl = pgurl.to_string()
        };
        if let Some(maps_api_key) = option_env!("MAPS_API_KEY") {
            self.maps_api_key = maps_api_key.to_string()
        };
        if let Some(gps_bucket) = option_env!("GPS_BUCKET") {
            self.gps_bucket = gps_bucket.to_string()
        };
        if let Some(cache_bucket) = option_env!("CACHE_BUCKET") {
            self.cache_bucket = cache_bucket.to_string()
        };
        if let Some(http_bucket) = option_env!("HTTP_BUCKET") {
            self.http_bucket = http_bucket.to_string()
        };
        if let Some(gps_dir) = option_env!("GPS_DIR") {
            self.gps_dir = gps_dir.to_string()
        };
        if let Some(cache_dir) = option_env!("CACHE_DIR") {
            self.cache_dir = cache_dir.to_string()
        };
        self
    }

    pub fn from_yml(mut self, filename: &str) -> GarminConfig {
        let settings = config::Config::new()
            .merge(config::File::with_name(filename))
            .unwrap()
            .clone();

        if let Ok(pgurl) = settings.get_str("PGURL") {
            self.pgurl = pgurl.to_string()
        };
        if let Ok(maps_api_key) = settings.get_str("MAPS_API_KEY") {
            self.maps_api_key = maps_api_key.to_string()
        };
        if let Ok(gps_bucket) = settings.get_str("GPS_BUCKET") {
            self.gps_bucket = gps_bucket.to_string()
        };
        if let Ok(cache_bucket) = settings.get_str("CACHE_BUCKET") {
            self.cache_bucket = cache_bucket.to_string()
        };
        if let Ok(http_bucket) = settings.get_str("HTTP_BUCKET") {
            self.http_bucket = http_bucket.to_string()
        };
        if let Ok(gps_dir) = settings.get_str("GPS_DIR") {
            self.gps_dir = gps_dir.to_string()
        };
        if let Ok(cache_dir) = settings.get_str("CACHE_DIR") {
            self.cache_dir = cache_dir.to_string()
        };
        self
    }
}
