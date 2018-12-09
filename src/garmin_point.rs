use std::fmt;

use crate::utils::garmin_util::{convert_xml_local_time_to_utc, METERS_PER_MILE};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminPoint {
    pub time: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub distance: Option<f64>,
    pub heart_rate: Option<f64>,
    pub duration_from_last: f64,
    pub duration_from_begin: f64,
    pub speed_mps: f64,
    pub speed_permi: f64,
    pub speed_mph: f64,
    pub avg_speed_value_permi: f64,
    pub avg_speed_value_mph: f64,
}

impl GarminPoint {
    pub fn new() -> GarminPoint {
        GarminPoint {
            time: "".to_string(),
            latitude: None,
            longitude: None,
            altitude: None,
            distance: None,
            heart_rate: None,
            duration_from_last: 0.0,
            duration_from_begin: 0.0,
            speed_mps: 0.0,
            speed_permi: 0.0,
            speed_mph: 0.0,
            avg_speed_value_permi: 0.0,
            avg_speed_value_mph: 0.0,
        }
    }

    pub fn clear(&mut self) {
        self.time = "".to_string();
        self.latitude = None;
        self.longitude = None;
        self.altitude = None;
        self.distance = None;
        self.heart_rate = None;
        self.duration_from_last = 0.0;
        self.duration_from_begin = 0.0;
        self.speed_mps = 0.0;
        self.speed_permi = 0.0;
        self.speed_mph = 0.0;
        self.avg_speed_value_permi = 0.0;
        self.avg_speed_value_mph = 0.0;
    }

    pub fn read_point_xml(&mut self, entries: &[&str]) {
        for entry in entries {
            let val = match entry.split("=").last() {
                Some(x) => x,
                None => continue,
            };
            if entry.contains("@time") {
                self.time =
                    convert_xml_local_time_to_utc(val).expect("Failed to parse time string");
            } else if entry.contains("@lat") {
                self.latitude = match val.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("@lon") {
                self.longitude = match val.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("@alt") {
                self.altitude = match val.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("@distance") {
                self.distance = match val.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("@hr") {
                self.heart_rate = match val.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            }
        }
    }

    pub fn read_point_tcx(&mut self, entries: &[&str]) {
        if let Some(&v0) = entries.get(0) {
            if v0.contains("Time") {
                self.time =
                    convert_xml_local_time_to_utc(v0.split("=").last().expect("Malformed time"))
                        .expect("Failed to read time");
            } else if v0.contains("Position") {
                if let Some(&v1) = entries.get(1) {
                    if v1.contains("LatitudeDegrees") {
                        self.latitude = match v1.split("=").last() {
                            Some(v) => match v.parse() {
                                Ok(x) => Some(x),
                                Err(_) => None,
                            },
                            None => None,
                        };
                    } else if v1.contains("LongitudeDegrees") {
                        self.longitude = match v1.split("=").last() {
                            Some(v) => match v.parse() {
                                Ok(x) => Some(x),
                                Err(_) => None,
                            },
                            None => None,
                        };
                    }
                }
            } else if v0.contains("AltitudeMeters") {
                self.altitude = match v0.split("=").last() {
                    Some(v) => match v.parse() {
                        Ok(x) => Some(x),
                        Err(_) => None,
                    },
                    None => None,
                };
            } else if v0.contains("DistanceMeters") {
                self.distance = match v0.split("=").last() {
                    Some(v) => match v.parse() {
                        Ok(x) => Some(x),
                        Err(_) => None,
                    },
                    None => None,
                };
            } else if v0.contains("HeartRateBpm") {
                if let Some(&v1) = entries.get(1) {
                    if v1.contains("Value") {
                        self.heart_rate = match v1.split("=").last() {
                            Some(v) => match v.parse() {
                                Ok(x) => Some(x),
                                Err(_) => None,
                            },
                            None => None,
                        };
                    }
                }
            } else if v0.contains("Extensions") {
                if let Some(&v2) = entries.get(2) {
                    if v2.contains("Speed") {
                        self.speed_mps = match v2.split("=").last() {
                            Some(v) => v.parse().unwrap_or(0.0),
                            None => 0.0,
                        };
                        self.speed_mph = self.speed_mps * 3600.0 / METERS_PER_MILE;
                        if self.speed_mps > 0.0 {
                            self.speed_permi = METERS_PER_MILE / self.speed_mps / 60.0;
                        }
                    }
                }
            }
        }
    }
}

impl fmt::Display for GarminPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let keys = vec![
            "time",
            "latitude",
            "longitude",
            "altitude",
            "distance",
            "heart_rate",
            "duration_from_last",
            "duration_from_begin",
            "speed_mps",
            "speed_permi",
            "speed_mph",
            "avg_speed_value_permi",
        ];
        let vals = vec![
            self.time.to_string(),
            self.latitude.unwrap_or(-1.0).to_string(),
            self.longitude.unwrap_or(-1.0).to_string(),
            self.altitude.unwrap_or(-1.0).to_string(),
            self.distance.unwrap_or(-1.0).to_string(),
            self.heart_rate.unwrap_or(-1.0).to_string(),
            self.duration_from_last.to_string(),
            self.duration_from_begin.to_string(),
            self.speed_mps.to_string(),
            self.speed_permi.to_string(),
            self.speed_mph.to_string(),
            self.avg_speed_value_permi.to_string(),
        ];
        write!(
            f,
            "GarminPoint<{}>",
            keys.iter()
                .zip(vals.iter())
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

pub const GARMIN_POINT_AVRO_SCHEMA: &str = r#"
    {
        "namespace": "garmin.avro",
        "type": "record",
        "name": "GarminPoint",
        "fields": [
            {"name": "time", "type": "string"},
            {"name": "latitude", "type": ["double", "null"]},
            {"name": "longitude", "type": ["double", "null"]},
            {"name": "altitude", "type": ["double", "null"]},
            {"name": "distance", "type": ["double", "null"]},
            {"name": "heart_rate", "type": ["double", "null"]},
            {"name": "duration_from_last", "type": "double"},
            {"name": "duration_from_begin", "type": "double"},
            {"name": "speed_mps", "type": "double"},
            {"name": "speed_permi", "type": "double"},
            {"name": "speed_mph", "type": "double"},
            {"name": "avg_speed_value_permi", "type": "double"},
            {"name": "avg_speed_value_mph", "type": "double"}
        ]
    }
"#;
