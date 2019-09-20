use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use chrono::{DateTime, Utc};
use failure::{err_msg, Error};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;

use crate::utils::iso_8601_datetime::{self, sentinel_datetime};
use crate::utils::sport_types::{self, SportTypes};

use super::garmin_lap::{GarminLap, GARMIN_LAP_AVRO_SCHEMA};
use super::garmin_point::{GarminPoint, GARMIN_POINT_AVRO_SCHEMA};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminFile {
    pub filename: String,
    pub filetype: String,
    #[serde(with = "iso_8601_datetime")]
    pub begin_datetime: DateTime<Utc>,
    #[serde(with = "sport_types")]
    pub sport: SportTypes,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub laps: Vec<GarminLap>,
    pub points: Vec<GarminPoint>,
}

impl Default for GarminFile {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminFile {
    pub fn new() -> GarminFile {
        GarminFile {
            filename: "".into(),
            filetype: "".into(),
            begin_datetime: sentinel_datetime(),
            sport: SportTypes::None,
            total_calories: 0,
            total_distance: 0.0,
            total_duration: 0.0,
            total_hr_dur: 0.0,
            total_hr_dis: 0.0,
            laps: Vec::new(),
            points: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.filename = "".into();
        self.filetype = "".into();
        self.begin_datetime = sentinel_datetime();
        self.sport = SportTypes::None;
        self.total_calories = 0;
        self.total_distance = 0.0;
        self.total_duration = 0.0;
        self.total_hr_dur = 0.0;
        self.total_hr_dis = 0.0;
        self.laps = Vec::new();
        self.points = Vec::new();
    }

    fn get_avro_schema() -> String {
        r#"{
            "namespace": "garmin.avro",
            "type": "record",
            "name": "GarminFile",
            "fields": [
                {"name": "filename", "type": "string"},
                {"name": "filetype", "type": "string"},
                {"name": "begin_datetime", "type": "string"},
                {"name": "sport", "type": "string"},
                {"name": "total_calories", "type": "int"},
                {"name": "total_distance", "type": "double"},
                {"name": "total_duration", "type": "double"},
                {"name": "total_hr_dur", "type": "double"},
                {"name": "total_hr_dis", "type": "double"},
                {"name": "laps", "type": {"type": "array", "items":"#
            .to_string()
            + &GARMIN_LAP_AVRO_SCHEMA.to_string()
            + r#"}},
                {"name": "points", "type": {"type": "array", "items": "#
            + &GARMIN_POINT_AVRO_SCHEMA.to_string()
            + r#"}}]
            }
        "#
    }

    pub fn dump_avro(&self, output_filename: &str) -> Result<(), Error> {
        let garmin_file_avro_schema = GarminFile::get_avro_schema();
        let schema = Schema::parse_str(&garmin_file_avro_schema)?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer.append_ser(&self)?;
        writer.flush().map(|_| ())
    }

    pub fn read_avro(input_filename: &str) -> Result<GarminFile, Error> {
        let garmin_file_avro_schema = GarminFile::get_avro_schema();
        let schema = Schema::parse_str(&garmin_file_avro_schema)?;

        let input_file = File::open(input_filename)?;

        let mut reader = Reader::with_schema(&schema, input_file)?;

        if let Some(record) = reader.next() {
            return match from_value::<GarminFile>(&record?) {
                Ok(v) => Ok(v),
                Err(e) => Err(err_msg(e)),
            };
        }
        Err(err_msg("Failed to find file"))
    }

    pub fn get_standardized_name(&self) -> Result<String, Error> {
        Ok(self
            .begin_datetime
            .format("%Y-%m-%d_%H-%M-%S_1_1.fit")
            .to_string())
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum GarminFileTypes {
    Txt,
    Tcx,
    Fit,
    Gpx,
    Gmn,
}

pub fn get_file_type_map() -> HashMap<String, GarminFileTypes> {
    [
        ("txt", GarminFileTypes::Txt),
        ("tcx", GarminFileTypes::Tcx),
        ("fit", GarminFileTypes::Fit),
        ("gpx", GarminFileTypes::Gpx),
        ("gmn", GarminFileTypes::Gmn),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

pub fn get_reverse_file_type_map() -> HashMap<GarminFileTypes, String> {
    get_file_type_map()
        .into_iter()
        .map(|(k, v)| (v, k))
        .collect()
}
