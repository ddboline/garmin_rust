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
            r"}}]}",
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

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use approx::assert_abs_diff_eq;
    use std::{
        io::{stdout, Write},
        path::Path,
    };

    use crate::{
        common::{garmin_correction_lap::GarminCorrectionLap, garmin_file},
        parsers::{garmin_parse::GarminParseTrait, garmin_parse_fit},
    };

    #[test]
    #[ignore]
    fn test_garmin_file_test_avro() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")?;
        let gfile = garmin_parse_fit::GarminParseFit::new()
            .with_file(Path::new("../tests/data/test.fit"), &corr_map)?;
        match gfile.dump_avro(Path::new("temp.avro")) {
            Ok(()) => {
                writeln!(stdout(), "Success")?;
            }
            Err(e) => {
                writeln!(stdout(), "{}", e)?;
            }
        }

        match garmin_file::GarminFile::read_avro(Path::new("temp.avro")) {
            Ok(g) => {
                writeln!(stdout(), "Success")?;
                assert_eq!(gfile.sport, g.sport);
                assert_eq!(gfile.filename, g.filename);
                assert_eq!(gfile.sport, g.sport);
                assert_eq!(gfile.filetype, g.filetype);
                assert_eq!(gfile.begin_datetime, g.begin_datetime);
                assert_eq!(gfile.total_calories, g.total_calories);
                assert_eq!(gfile.laps.len(), g.laps.len());
                assert_eq!(gfile.points.len(), g.points.len());
                assert_abs_diff_eq!(gfile.total_distance, g.total_distance);
                assert_abs_diff_eq!(gfile.total_duration, g.total_duration);
                assert_abs_diff_eq!(gfile.total_hr_dur, g.total_hr_dur);
                assert_abs_diff_eq!(gfile.total_hr_dis, g.total_hr_dis);
            }
            Err(e) => {
                writeln!(stdout(), "{}", e)?;
                assert!(false);
            }
        }

        std::fs::remove_file("temp.avro")?;
        Ok(())
    }
}
