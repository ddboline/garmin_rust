use anyhow::{format_err, Error};
use fitparser::{FitDataField, Value};
use itertools::Itertools;
use roxmltree::{Node, NodeType};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::fmt;
use time::OffsetDateTime;
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};

use garmin_lib::date_time_wrapper::DateTimeWrapper;
use garmin_utils::garmin_util::{
    convert_xml_local_time_to_utc, get_degrees_from_semicircles, get_f64, METERS_PER_MILE,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct GarminPoint {
    pub time: DateTimeWrapper,
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            time: DateTimeWrapper::sentinel_datetime(),
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
        self.time = DateTimeWrapper::sentinel_datetime();
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

    /// # Errors
    /// Return error in `convert_xml_local_time_to_utc` fails
    pub fn read_point_xml(entries: &Node) -> Result<Self, Error> {
        let mut new_point = Self::new();
        for entry in entries.attributes() {
            match entry.name() {
                "time" => new_point.time = convert_xml_local_time_to_utc(entry.value())?.into(),
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

    /// # Errors
    /// Return error if `convert_xml_local_time_to_utc` fails
    pub fn read_point_tcx(entries: &Node) -> Result<Self, Error> {
        let mut new_point = Self::new();
        for d in entries.descendants() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "Time" => {
                        new_point.time = convert_xml_local_time_to_utc(
                            d.text().ok_or_else(|| format_err!("Malformed time"))?,
                        )?
                        .into();
                    }
                    "AltitudeMeters" => new_point.altitude = d.text().and_then(|x| x.parse().ok()),
                    "LatitudeDegrees" => new_point.latitude = d.text().and_then(|x| x.parse().ok()),
                    "LongitudeDegrees" => {
                        new_point.longitude = d.text().and_then(|x| x.parse().ok());
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

    #[must_use]
    pub fn read_point_fit(fields: &[FitDataField]) -> Self {
        let mut new_point = Self::new();
        for field in fields {
            match field.name() {
                "timestamp" => {
                    if let Value::Timestamp(t) = field.value() {
                        new_point.time = t.to_timezone(UTC).into();
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
        new_point
    }

    pub fn calculate_durations(point_list: &mut [Self]) {
        let mut time_from_begin = 0.0;
        let mut last_time = None;
        for point in &mut *point_list {
            let point_time: OffsetDateTime = point.time.into();
            let duration_from_last = match last_time.replace(point_time) {
                None => 0.0,
                Some(last_time) => (point_time - last_time).whole_seconds() as f64,
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
        let keys = [
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
            StackString::from_display(self.time),
            StackString::from_display(self.latitude.unwrap_or(-1.0)),
            StackString::from_display(self.longitude.unwrap_or(-1.0)),
            StackString::from_display(self.altitude.unwrap_or(-1.0)),
            StackString::from_display(self.distance.unwrap_or(-1.0)),
            StackString::from_display(self.heart_rate.unwrap_or(-1.0)),
            StackString::from_display(self.duration_from_last),
            StackString::from_display(self.duration_from_begin),
            StackString::from_display(self.speed_mps),
            StackString::from_display(self.speed_permi),
            StackString::from_display(self.speed_mph),
            StackString::from_display(self.avg_speed_value_permi),
        ];
        write!(
            f,
            "GarminPoint<{}>",
            keys.iter()
                .zip(vals.iter())
                .map(|(k, v)| { format_sstr!("{k}={v}") })
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
            {"name": "latitude", "type": ["null", "double"]},
            {"name": "longitude", "type": ["null", "double"]},
            {"name": "altitude", "type": ["null", "double"]},
            {"name": "distance", "type": ["null", "double"]},
            {"name": "heart_rate", "type": ["null", "double"]},
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
