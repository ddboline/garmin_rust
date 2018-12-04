extern crate flate2;

use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use failure::{err_msg, Error};

use crate::garmin_correction_lap::GarminCorrectionLap;
use crate::garmin_lap::{GarminLap, GARMIN_LAP_AVRO_SCHEMA};
use crate::garmin_point::{GarminPoint, GARMIN_POINT_AVRO_SCHEMA};
use crate::garmin_util::METERS_PER_MILE;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminFile {
    pub filename: String,
    pub filetype: String,
    pub begin_datetime: String,
    pub sport: Option<String>,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub laps: Vec<GarminLap>,
    pub points: Vec<GarminPoint>,
}

impl GarminFile {
    pub fn new() -> GarminFile {
        GarminFile {
            filename: "".to_string(),
            filetype: "".to_string(),
            begin_datetime: "".to_string(),
            sport: None,
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
        self.filename = "".to_string();
        self.filetype = "".to_string();
        self.begin_datetime = "".to_string();
        self.sport = None;
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
                {"name": "sport", "type": ["string", "null"]},
                {"name": "total_calories", "type": "int"},
                {"name": "total_distance", "type": "double"},
                {"name": "total_duration", "type": "double"},
                {"name": "total_hr_dur", "type": "double"},
                {"name": "total_hr_dis", "type": "double"},
                {"name": "laps", "type": {"type": "array", "items":"#.to_string()
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
        writer.flush()?;

        Ok(())
    }

    pub fn read_avro(input_filename: &str) -> Result<GarminFile, Error> {
        let garmin_file_avro_schema = GarminFile::get_avro_schema();
        let schema = Schema::parse_str(&garmin_file_avro_schema)?;

        let input_file = File::open(input_filename)?;

        let reader = Reader::with_schema(&schema, input_file)?;

        for record in reader {
            return match from_value::<GarminFile>(&record?) {
                Ok(v) => Ok(v),
                Err(e) => Err(err_msg(e)),
            };
        }
        Err(err_msg("Failed to find file"))
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
    ].iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

pub fn get_reverse_file_type_map() -> HashMap<GarminFileTypes, String> {
    get_file_type_map()
        .into_iter()
        .map(|(k, v)| (v, k))
        .collect()
}

pub fn apply_lap_corrections(
    lap_list: Vec<GarminLap>,
    sport: Option<String>,
    corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
) -> (Vec<GarminLap>, Option<String>) {
    let mut new_sport = sport.clone();
    match lap_list.get(0) {
        Some(l) => {
            let lap_start = &l.lap_start.clone();
            for lap in &lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            let new_lap_list: Vec<_> = lap_list
                .into_iter()
                .map(|lap| {
                    let lap_number = lap.lap_number;
                    match &corr_map.get(&(lap_start.to_string(), lap_number)) {
                        Some(corr) => {
                            let mut new_lap = lap.clone();
                            new_sport = match &corr.sport {
                                Some(s) => {
                                    debug!("change sport {} {:?} {}", lap_start, lap.lap_type, s);
                                    Some(s.clone())
                                }
                                None => sport.clone(),
                            };
                            new_lap.lap_duration = match &corr.duration {
                                Some(dur) => {
                                    debug!(
                                        "change duration {} {} {}",
                                        lap_start, lap.lap_duration, dur
                                    );
                                    dur.clone()
                                }
                                None => lap.lap_duration.clone(),
                            };
                            new_lap.lap_distance = match &corr.distance {
                                Some(dis) => {
                                    debug!(
                                        "change duration {} {} {}",
                                        lap_start,
                                        lap.lap_distance,
                                        dis * METERS_PER_MILE
                                    );
                                    dis * METERS_PER_MILE
                                }
                                None => lap.lap_distance.clone(),
                            };
                            new_lap
                        }
                        None => lap.clone(),
                    }
                })
                .collect();
            for lap in &new_lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            (new_lap_list, new_sport)
        }
        None => (Vec::new(), new_sport),
    }
}

pub fn check_cached_files() -> Vec<String> {
    let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

    let path = Path::new(cache_dir);

    match path.read_dir() {
        Ok(it) => it.filter_map(|dir_line| match dir_line {
            Ok(entry) => {
                let input_file = entry.path().to_str().unwrap().to_string();
                println!("{}", input_file);
                let gfile = GarminFile::read_avro(&input_file).unwrap();
                println!("{}", gfile.points.len());
                Some(input_file)
            }
            Err(_) => None,
        }).collect(),
        Err(err) => {
            println!("{}", err);
            Vec::new()
        }
    }
}
