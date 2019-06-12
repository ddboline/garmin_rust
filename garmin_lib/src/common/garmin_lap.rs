use failure::Error;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};

use std::fmt;

use crate::utils::garmin_util::{convert_time_string, convert_xml_local_time_to_utc};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GarminLap {
    pub lap_type: Option<String>,
    pub lap_index: i32,
    pub lap_start: String,
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

impl GarminLap {
    pub fn new() -> GarminLap {
        GarminLap {
            lap_type: None,
            lap_index: -1,
            lap_start: "".to_string(),
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
        self.lap_start = "".to_string();
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

    pub fn read_lap_xml(&mut self, entries: &[&str]) -> Result<(), Error> {
        for entry in entries {
            let value = match entry.split('=').last() {
                Some(x) => x,
                None => continue,
            };
            if entry.contains("@type") {
                self.lap_type = Some(value.to_string());
            } else if entry.contains("index") {
                self.lap_index = value.parse().unwrap_or(-1);
            } else if entry.contains("start") {
                self.lap_start = convert_xml_local_time_to_utc(value)?;
                self.lap_start_string = Some(self.lap_start.clone());
            } else if entry.contains("duration") {
                self.lap_duration = convert_time_string(value).unwrap_or(0.0);
            } else if entry.contains("distance") {
                self.lap_distance = value.parse().unwrap_or(0.0);
            } else if entry.contains("trigger") {
                self.lap_trigger = Some(value.to_string());
            } else if entry.contains("max_speed") {
                self.lap_max_speed = match value.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("calories") {
                self.lap_calories = value.parse().unwrap_or(0);
            } else if entry.contains("avg_hr") {
                self.lap_max_hr = match value.parse() {
                    Ok(x) => Some(x),
                    Err(_) => None,
                };
            } else if entry.contains("intensity") {
                self.lap_intensity = Some(value.to_string());
            }
        }
        Ok(())
    }

    pub fn read_lap_tcx(&mut self, entries: &[&str]) -> Result<(), Error> {
        if let Some(&v0) = entries.get(0) {
            if let Some(value) = v0.split('=').last() {
                if v0.contains("@StartTime") {
                    self.lap_start = convert_xml_local_time_to_utc(value)?;
                    self.lap_start_string = Some(self.lap_start.clone());
                } else if v0.contains("TotalTimeSeconds") {
                    self.lap_duration = value.parse().unwrap_or(0.0);
                } else if v0.contains("DistanceMeters") {
                    self.lap_distance = value.parse().unwrap_or(0.0);
                } else if v0.contains("TriggerMethod") {
                    self.lap_trigger = Some(value.to_string());
                } else if v0.contains("MaximumSpeed") {
                    self.lap_max_speed = match value.parse() {
                        Ok(x) => Some(x),
                        Err(_) => None,
                    };
                } else if v0.contains("Calories") {
                    self.lap_calories = value.parse().unwrap_or(0);
                } else if v0.contains("Intensity") {
                    self.lap_intensity = Some(value.to_string());
                } else if v0.contains("AverageHeartRateBpm") {
                    if let Some(&v1) = entries.get(1) {
                        if v1.contains("Value") {
                            self.lap_avg_hr = match v1.split('=').last() {
                                Some(val) => match val.parse() {
                                    Ok(x) => Some(x),
                                    Err(_) => None,
                                },
                                None => None,
                            };
                        }
                    }
                } else if v0.contains("MaximumHeartRateBpm") {
                    if let Some(&v1) = entries.get(1) {
                        if v1.contains("Value") {
                            self.lap_max_hr = match v1.split('=').last() {
                                Some(val) => match val.parse() {
                                    Ok(x) => Some(x),
                                    Err(_) => None,
                                },
                                None => None,
                            };
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn fix_lap_number(lap_list: Vec<GarminLap>) -> Vec<GarminLap> {
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
        let vals = vec![
            self.lap_type.clone().unwrap_or_else(|| "None".to_string()),
            self.lap_index.to_string(),
            self.lap_start.to_string(),
            self.lap_duration.to_string(),
            self.lap_distance.to_string(),
            self.lap_trigger
                .clone()
                .unwrap_or_else(|| "None".to_string()),
            self.lap_max_speed.unwrap_or(-1.0).to_string(),
            self.lap_calories.to_string(),
            self.lap_avg_hr.unwrap_or(-1.0).to_string(),
            self.lap_max_hr.unwrap_or(-1).to_string(),
            self.lap_intensity
                .clone()
                .unwrap_or_else(|| "None".to_string()),
            self.lap_number.to_string(),
            self.lap_start_string
                .clone()
                .unwrap_or_else(|| "None".to_string()),
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
