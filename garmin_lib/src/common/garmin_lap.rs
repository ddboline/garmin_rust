use anyhow::Error;
use chrono::{DateTime, Utc};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use roxmltree::{Node, NodeType};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::utils::garmin_util::{convert_time_string, convert_xml_local_time_to_utc};
use crate::utils::iso_8601_datetime::{self, convert_datetime_to_str, sentinel_datetime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminLap {
    pub lap_type: Option<String>,
    pub lap_index: i32,
    #[serde(with = "iso_8601_datetime")]
    pub lap_start: DateTime<Utc>,
    pub lap_duration: f64,
    pub lap_distance: f64,
    pub lap_trigger: Option<String>,
    pub lap_max_speed: Option<f64>,
    pub lap_calories: i32,
    pub lap_avg_hr: Option<f64>,
    pub lap_max_hr: Option<i32>,
    pub lap_intensity: Option<String>,
    pub lap_number: i32,
    pub lap_start_string: Option<String>,
}

impl Default for GarminLap {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminLap {
    pub fn new() -> Self {
        Self {
            lap_type: None,
            lap_index: -1,
            lap_start: sentinel_datetime(),
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
        self.lap_start = sentinel_datetime();
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

    pub fn read_lap_xml(entries: &Node) -> Result<Self, Error> {
        let mut new_lap = Self::new();
        for d in entries.descendants() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "max_speed" => new_lap.lap_max_speed = d.text().and_then(|x| x.parse().ok()),
                    "calories" => {
                        new_lap.lap_calories = d.text().and_then(|x| x.parse().ok()).unwrap_or(0)
                    }
                    "intensity" => new_lap.lap_intensity = d.text().map(ToString::to_string),
                    _ => (),
                }
            }
        }
        for entry in entries.attributes() {
            match entry.name() {
                "type" => new_lap.lap_type = Some(entry.value().to_string()),
                "index" => new_lap.lap_index = entry.value().parse().unwrap_or(-1),
                "start" => {
                    new_lap.lap_start = convert_xml_local_time_to_utc(entry.value())?;
                    new_lap.lap_start_string = Some(convert_datetime_to_str(new_lap.lap_start));
                }
                "duration" => {
                    new_lap.lap_duration = convert_time_string(entry.value()).unwrap_or(0.0)
                }
                "distance" => new_lap.lap_distance = entry.value().parse().unwrap_or(0.0),
                "trigger" => new_lap.lap_trigger = Some(entry.value().to_string()),
                "avg_hr" => new_lap.lap_max_hr = entry.value().parse().ok(),
                _ => (),
            }
        }
        Ok(new_lap)
    }

    pub fn read_lap_tcx(entries: &Node) -> Result<Self, Error> {
        let mut new_lap = Self::new();
        for d in entries.children() {
            if d.node_type() == NodeType::Element {
                match d.tag_name().name() {
                    "TotalTimeSeconds" => {
                        new_lap.lap_duration = d.text().and_then(|x| x.parse().ok()).unwrap_or(0.0)
                    }
                    "DistanceMeters" => {
                        new_lap.lap_distance = d.text().and_then(|x| x.parse().ok()).unwrap_or(0.0)
                    }
                    "MaximumSpeed" => new_lap.lap_max_speed = d.text().and_then(|x| x.parse().ok()),
                    "TriggerMethod" => new_lap.lap_trigger = d.text().map(ToString::to_string),
                    "Calories" => {
                        new_lap.lap_calories = d.text().and_then(|x| x.parse().ok()).unwrap_or(0)
                    }
                    "Intensity" => new_lap.lap_intensity = d.text().map(ToString::to_string),
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
                new_lap.lap_start = convert_xml_local_time_to_utc(entry.value())?;
                new_lap.lap_start_string = Some(convert_datetime_to_str(new_lap.lap_start));
            }
        }
        Ok(new_lap)
    }

    pub fn fix_lap_number(lap_list: Vec<Self>) -> Vec<Self> {
        lap_list
            .into_par_iter()
            .enumerate()
            .map(|(i, lap)| {
                let mut new_lap = lap;
                new_lap.lap_index = i as i32;
                new_lap.lap_number = if new_lap.lap_number == -1 {
                    i as i32
                } else {
                    new_lap.lap_number
                };
                new_lap
            })
            .collect()
    }
}

impl fmt::Display for GarminLap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let lap_type = self.lap_type.as_ref().map_or("None", String::as_str);
        let lap_trigger = self.lap_trigger.as_ref().map_or("None", String::as_str);
        let lap_intensity = self.lap_intensity.as_ref().map_or("None", String::as_str);
        let lap_start_string = self
            .lap_start_string
            .as_ref()
            .map_or("None", String::as_str);
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
                .map(|(k, v)| format!("{}={}", k, v))
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
