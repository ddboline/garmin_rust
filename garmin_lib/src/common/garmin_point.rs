use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use fitparser::{FitDataField, Value};
use itertools::Itertools;
use roxmltree::{Node, NodeType};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::utils::{
    garmin_util::{
        convert_xml_local_time_to_utc, get_degrees_from_semicircles, get_f64, METERS_PER_MILE,
    },
    iso_8601_datetime::{self, sentinel_datetime},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GarminPoint {
    #[serde(with = "iso_8601_datetime")]
    pub time: DateTime<Utc>,
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

impl Default for GarminPoint {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminPoint {
    pub fn new() -> Self {
        Self {
            time: sentinel_datetime(),
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
        self.time = sentinel_datetime();
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

    pub fn read_point_xml(entries: &Node) -> Result<Self, Error> {
        let mut new_point = Self::new();
        for entry in entries.attributes() {
            match entry.name() {
                "time" => new_point.time = convert_xml_local_time_to_utc(entry.value())?,
                "lat" => new_point.latitude = entry.value().parse().ok(),
                "lon" => new_point.longitude = entry.value().parse().ok(),
                "alt" => new_point.altitude = entry.value().parse().ok(),
                "distance" => new_point.distance = entry.value().parse().ok(),
                "hr" => new_point.heart_rate = entry.value().parse().ok(),
                _ => (),
            }
        }
        Ok(new_point)
    }

    pub fn read_point_tcx(entries: &Node) -> Result<Self, Error> {
        let mut new_point = Self::new();
        for d in entries.descendants() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "Time" => {
                        new_point.time = convert_xml_local_time_to_utc(
                            d.text().ok_or_else(|| format_err!("Malformed time"))?,
                        )?
                    }
                    "AltitudeMeters" => new_point.altitude = d.text().and_then(|x| x.parse().ok()),
                    "LatitudeDegrees" => new_point.latitude = d.text().and_then(|x| x.parse().ok()),
                    "LongitudeDegrees" => {
                        new_point.longitude = d.text().and_then(|x| x.parse().ok())
                    }
                    "DistanceMeters" => new_point.distance = d.text().and_then(|x| x.parse().ok()),
                    "HeartRateBpm" => {
                        for entry in d.descendants() {
                            if entry.node_type() == NodeType::Element
                                && entry.tag_name().name() == "Value"
                            {
                                new_point.heart_rate = entry.text().and_then(|x| x.parse().ok());
                            }
                        }
                    }
                    "Extensions" => {
                        for entry in d.descendants() {
                            if entry.node_type() == NodeType::Element
                                && entry.tag_name().name() == "Speed"
                            {
                                new_point.speed_mps =
                                    entry.text().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                                new_point.speed_mph =
                                    new_point.speed_mps * 3600.0 / METERS_PER_MILE;
                                if new_point.speed_mps > 0.0 {
                                    new_point.speed_permi =
                                        METERS_PER_MILE / new_point.speed_mps / 60.0;
                                }
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
        Ok(new_point)
    }

    pub fn read_point_fit(fields: &[FitDataField]) -> Result<Self, Error> {
        let mut new_point = Self::new();
        for field in fields {
            match field.name() {
                "timestamp" => {
                    if let Value::Timestamp(t) = field.value() {
                        new_point.time = t.with_timezone(&Utc);
                    }
                }
                "enhanced_altitude" => {
                    new_point.altitude = get_f64(field.value());
                }
                "position_lat" => {
                    new_point.latitude = get_f64(field.value()).map(get_degrees_from_semicircles);
                }
                "position_long" => {
                    new_point.longitude = get_f64(field.value()).map(get_degrees_from_semicircles);
                }
                "distance" => {
                    new_point.distance = get_f64(field.value());
                }
                "heart_rate" => {
                    new_point.heart_rate = get_f64(field.value());
                }
                "enhanced_speed" => {
                    if let Some(f) = get_f64(field.value()) {
                        new_point.speed_mps = f;
                        new_point.speed_mph = new_point.speed_mps * 3600.0 / METERS_PER_MILE;
                        if new_point.speed_mps > 0.0 {
                            new_point.speed_permi = METERS_PER_MILE / new_point.speed_mps / 60.0;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(new_point)
    }

    pub fn calculate_durations(point_list: &mut [Self]) {
        let mut time_from_begin = 0.0;
        let mut last_time = None;
        for point in point_list.iter_mut() {
            let duration_from_last = match last_time.replace(point.time) {
                None => 0.0,
                Some(last_time) => (point.time - last_time).num_seconds() as f64,
            };
            time_from_begin += duration_from_last;
            let duration_from_begin = time_from_begin;
            point.duration_from_begin = duration_from_begin;
            point.duration_from_last = duration_from_last;
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
