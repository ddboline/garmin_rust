use anyhow::Error;
use fitparser::{FitDataField, Value};
use roxmltree::{Node, NodeType};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::fmt;
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};

use crate::utils::{
    date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
    garmin_util::{convert_time_string, convert_xml_local_time_to_utc, get_f64, get_i64},
    sport_types::SportTypes,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GarminLap {
    pub lap_type: Option<StackString>,
    pub lap_index: i32,
    pub lap_start: DateTimeWrapper,
    pub lap_duration: f64,
    pub lap_distance: f64,
    pub lap_trigger: Option<StackString>,
    pub lap_max_speed: Option<f64>,
    pub lap_calories: i32,
    pub lap_avg_hr: Option<f64>,
    pub lap_max_hr: Option<i32>,
    pub lap_intensity: Option<StackString>,
    pub lap_number: i32,
    pub lap_start_string: Option<StackString>,
}

impl Default for GarminLap {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminLap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lap_type: None,
            lap_index: -1,
            lap_start: DateTimeWrapper::sentinel_datetime(),
            lap_duration: 0.0,
            lap_distance: 0.0,
            lap_trigger: None,
            lap_max_speed: None,
            lap_calories: 0,
            lap_avg_hr: None,
            lap_max_hr: None,
            lap_intensity: None,
            lap_number: -1,
            lap_start_string: None,
        }
    }

    pub fn clear(&mut self) {
        self.lap_type = None;
        self.lap_index = -1;
        self.lap_start = DateTimeWrapper::sentinel_datetime();
        self.lap_duration = 0.0;
        self.lap_distance = 0.0;
        self.lap_trigger = None;
        self.lap_max_speed = None;
        self.lap_calories = 0;
        self.lap_avg_hr = None;
        self.lap_max_hr = None;
        self.lap_intensity = None;
        self.lap_number = -1;
        self.lap_start_string = None;
    }

    /// # Errors
    /// Return error if `convert_xml_local_time_to_utc` fails
    pub fn read_lap_xml(entries: &Node) -> Result<Self, Error> {
        let mut new_lap = Self::new();
        for d in entries.descendants() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "max_speed" => new_lap.lap_max_speed = d.text().and_then(|x| x.parse().ok()),
                    "calories" => {
                        new_lap.lap_calories = d.text().and_then(|x| x.parse().ok()).unwrap_or(0);
                    }
                    "intensity" => new_lap.lap_intensity = d.text().map(Into::into),
                    _ => (),
                }
            }
        }
        for entry in entries.attributes() {
            match entry.name() {
                "type" => new_lap.lap_type = Some(entry.value().into()),
                "index" => new_lap.lap_index = entry.value().parse().unwrap_or(-1),
                "start" => {
                    new_lap.lap_start = convert_xml_local_time_to_utc(entry.value())?.into();
                    new_lap.lap_start_string =
                        Some(convert_datetime_to_str(new_lap.lap_start.into()));
                }
                "duration" => {
                    new_lap.lap_duration = convert_time_string(entry.value()).unwrap_or(0.0);
                }
                "distance" => new_lap.lap_distance = entry.value().parse().unwrap_or(0.0),
                "trigger" => new_lap.lap_trigger = Some(entry.value().into()),
                "avg_hr" => new_lap.lap_max_hr = entry.value().parse().ok(),
                _ => (),
            }
        }
        Ok(new_lap)
    }

    /// # Errors
    /// Returns error if `convert_xml_local_time_to_utc` fails
    pub fn read_lap_tcx(entries: &Node) -> Result<Self, Error> {
        let mut new_lap = Self::new();
        for d in entries.children() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "TotalTimeSeconds" => {
                        new_lap.lap_duration = d.text().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    }
                    "DistanceMeters" => {
                        new_lap.lap_distance = d.text().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    }
                    "MaximumSpeed" => new_lap.lap_max_speed = d.text().and_then(|x| x.parse().ok()),
                    "TriggerMethod" => new_lap.lap_trigger = d.text().map(Into::into),
                    "Calories" => {
                        new_lap.lap_calories = d.text().and_then(|x| x.parse().ok()).unwrap_or(0);
                    }
                    "Intensity" => new_lap.lap_intensity = d.text().map(Into::into),
                    "AverageHeartRateBpm" => {
                        for entry in d.descendants() {
                            if entry.node_type() == NodeType::Element
                                && entry.tag_name().name() == "Value"
                            {
                                new_lap.lap_avg_hr = entry.text().and_then(|x| x.parse().ok());
                            }
                        }
                    }
                    "MaximumHeartRateBpm" => {
                        for entry in d.descendants() {
                            if entry.node_type() == NodeType::Element
                                && entry.tag_name().name() == "Value"
                            {
                                new_lap.lap_max_hr = entry.text().and_then(|x| x.parse().ok());
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
        for entry in entries.attributes() {
            if entry.name() == "StartTime" {
                new_lap.lap_start = convert_xml_local_time_to_utc(entry.value())?.into();
                new_lap.lap_start_string = Some(convert_datetime_to_str(new_lap.lap_start.into()));
            }
        }
        Ok(new_lap)
    }

    #[must_use]
    pub fn read_lap_fit(fields: &[FitDataField]) -> (Self, Option<SportTypes>) {
        let mut new_lap = Self::new();
        let mut lap_sport = None;
        for field in fields {
            match field.name() {
                "start_time" => {
                    if let Value::Timestamp(t) = field.value() {
                        new_lap.lap_start = t.to_timezone(UTC).into();
                        new_lap.lap_start_string =
                            Some(convert_datetime_to_str(new_lap.lap_start.into()));
                    }
                }
                "total_timer_time" => {
                    if let Some(f) = get_f64(field.value()) {
                        new_lap.lap_duration = f;
                    }
                }
                "total_distance" => {
                    if let Some(f) = get_f64(field.value()) {
                        new_lap.lap_distance = f;
                    }
                }
                "enhanced_avg_speed" => {
                    new_lap.lap_max_speed = get_f64(field.value());
                }
                "lap_trigger" => {
                    if let Value::String(s) = field.value() {
                        new_lap.lap_trigger = Some(s.into());
                    }
                }
                "total_calories" => {
                    if let Some(i) = get_i64(field.value()) {
                        new_lap.lap_calories = i as i32;
                    }
                }
                "avg_heart_rate" => {
                    new_lap.lap_avg_hr = get_f64(field.value());
                }
                "max_heart_rate" => {
                    if let Some(i) = get_i64(field.value()) {
                        new_lap.lap_max_hr = Some(i as i32);
                    }
                }
                "sport" => {
                    if let Value::String(s) = field.value() {
                        if let Ok(sport) = s.parse() {
                            lap_sport.replace(sport);
                        }
                    }
                }
                _ => {}
            }
        }
        (new_lap, lap_sport)
    }

    pub fn fix_lap_number(lap_list: &mut [Self]) {
        for (i, lap) in lap_list.iter_mut().enumerate() {
            lap.lap_index = i as i32;
            if lap.lap_number == -1 {
                lap.lap_number = i as i32;
            }
        }
    }
}

impl fmt::Display for GarminLap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let lap_type = self.lap_type.as_ref().map_or("None", StackString::as_str);
        let lap_trigger = self
            .lap_trigger
            .as_ref()
            .map_or("None", StackString::as_str);
        let lap_intensity = self
            .lap_intensity
            .as_ref()
            .map_or("None", StackString::as_str);
        let lap_start_string = self
            .lap_start_string
            .as_ref()
            .map_or("None", StackString::as_str);
        let lap_max_speed = self.lap_max_speed.unwrap_or(-1.0);
        let lap_avg_hr = self.lap_avg_hr.unwrap_or(-1.0);
        let lap_max_hr = self.lap_max_hr.unwrap_or(-1);
        let keys = vec![
            "lap_type",
            "lap_index",
            "lap_start",
            "lap_duration",
            "lap_distance",
            "lap_trigger",
            "lap_max_speed",
            "lap_calories",
            "lap_avg_hr",
            "lap_max_hr",
            "lap_intensity",
            "lap_number",
            "lap_start_string",
        ];
        let vals: Vec<&dyn fmt::Display> = vec![
            &lap_type,
            &self.lap_index,
            &self.lap_start,
            &self.lap_duration,
            &self.lap_distance,
            &lap_trigger,
            &lap_max_speed,
            &self.lap_calories,
            &lap_avg_hr,
            &lap_max_hr,
            &lap_intensity,
            &self.lap_number,
            &lap_start_string,
        ];
        write!(
            f,
            "GarminLap<{}>",
            keys.iter()
                .zip(vals.iter())
                .map(|(k, v)| { format_sstr!("{k}={v}") })
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

pub const GARMIN_LAP_AVRO_SCHEMA: &str = r#"
    {
        "namespace": "garmin.avro",
        "type": "record",
        "name": "GarminLap",
        "fields": [
            {"name": "lap_type", "type": ["string", "null"]},
            {"name": "lap_index", "type": "int"},
            {"name": "lap_start", "type": "string"},
            {"name": "lap_duration", "type": "double"},
            {"name": "lap_distance", "type": "double"},
            {"name": "lap_trigger", "type": ["string", "null"]},
            {"name": "lap_max_speed", "type": ["double", "null"]},
            {"name": "lap_calories", "type": "int"},
            {"name": "lap_avg_hr", "type": ["double", "null"]},
            {"name": "lap_max_hr", "type": ["int", "null"]},
            {"name": "lap_intensity", "type": ["string", "null"]},
            {"name": "lap_number", "type": "int"},
            {"name": "lap_start_string", "type": ["string", "null"]}
        ]
    }
"#;
