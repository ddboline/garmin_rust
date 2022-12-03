use anyhow::{format_err, Error};
use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fs::File, path::Path};
use time::macros::format_description;
use tokio::task::spawn_blocking;

use crate::utils::{date_time_wrapper::DateTimeWrapper, sport_types::SportTypes};

use super::{
    garmin_lap::{GarminLap, GARMIN_LAP_AVRO_SCHEMA},
    garmin_point::{GarminPoint, GARMIN_POINT_AVRO_SCHEMA},
};

lazy_static! {
    static ref GARMIN_FILE_AVRO_SCHEMA: StackString = GarminFile::get_avro_schema();
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GarminFile {
    pub filename: StackString,
    pub filetype: StackString,
    pub begin_datetime: DateTimeWrapper,
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            filename: "".into(),
            filetype: "".into(),
            begin_datetime: DateTimeWrapper::sentinel_datetime(),
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
        self.begin_datetime = DateTimeWrapper::sentinel_datetime();
        self.sport = SportTypes::None;
        self.total_calories = 0;
        self.total_distance = 0.0;
        self.total_duration = 0.0;
        self.total_hr_dur = 0.0;
        self.total_hr_dis = 0.0;
        self.laps = Vec::new();
        self.points = Vec::new();
    }

    fn get_avro_schema() -> StackString {
        format_sstr!(
            "{}{}{}{}{}",
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
                    {"name": "laps", "type": {"type": "array", "items":"#,
            GARMIN_LAP_AVRO_SCHEMA,
            r#"}},{"name": "points", "type": {"type": "array", "items": "#,
            GARMIN_POINT_AVRO_SCHEMA,
            r#"}}]}"#,
        )
    }

    /// # Errors
    /// Return error if `parse_str` fails, or opening file fails, or writing
    /// codec fails
    pub fn dump_avro(&self, output_filename: &Path) -> Result<(), Error> {
        let schema = Schema::parse_str(&GARMIN_FILE_AVRO_SCHEMA)?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);
        writer.append_ser(self)?;
        writer.flush()?;
        Ok(())
    }

    /// # Errors
    /// Return error if `read_avro` fails
    pub async fn read_avro_async(input_filename: &Path) -> Result<Self, Error> {
        let input_filename = input_filename.to_owned();
        spawn_blocking(move || Self::read_avro(&input_filename)).await?
    }

    /// # Errors
    /// Return error if open file fails, or reader fails
    pub fn read_avro(input_filename: &Path) -> Result<Self, Error> {
        if !input_filename.exists() {
            return Err(format_err!("file {input_filename:?} does not exist"));
        }
        let input_file = File::open(input_filename)?;

        let mut reader = Reader::new(input_file)?;

        if let Some(record) = reader.next() {
            return from_value::<Self>(&record?).map_err(Into::into);
        }
        Err(format_err!("Failed to find file"))
    }

    #[must_use]
    pub fn get_standardized_name(&self, suffix: &str) -> StackString {
        format_sstr!(
            "{d}.{suffix}",
            d = self
                .begin_datetime
                .format(format_description!(
                    "[year]-[month]-[day]_[hour]-[minute]-[second]-1-1"
                ))
                .unwrap_or_else(|_| String::new())
        )
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

#[must_use]
pub fn get_file_type_map() -> HashMap<StackString, GarminFileTypes> {
    [
        ("txt", GarminFileTypes::Txt),
        ("tcx", GarminFileTypes::Tcx),
        ("fit", GarminFileTypes::Fit),
        ("gpx", GarminFileTypes::Gpx),
        ("gmn", GarminFileTypes::Gmn),
    ]
    .iter()
    .map(|(k, v)| ((*k).into(), *v))
    .collect()
}

#[must_use]
pub fn get_reverse_file_type_map() -> HashMap<GarminFileTypes, StackString> {
    get_file_type_map()
        .into_iter()
        .map(|(k, v)| (v, k))
        .collect()
}
