extern crate chrono;
extern crate subprocess;

use failure::Error;
use std::collections::HashMap;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

use chrono::{DateTime, Utc};
use std::str::FromStr;

use crate::garmin_correction_lap::GarminCorrectionLap;
use crate::garmin_file::{apply_lap_corrections, GarminFile};
use crate::garmin_lap::GarminLap;
use crate::garmin_point::GarminPoint;
use crate::garmin_util::get_sport_type_map;

#[derive(Debug)]
pub struct GarminParseTcx {
    pub gfile: GarminFile,
}

impl GarminParseTcx {
    pub fn new(
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
        is_fit_file: bool,
    ) -> GarminParseTcx {
        let file_name = Path::new(&filename)
            .file_name()
            .expect(&format!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or(filename.to_string());
        let (lap_list, point_list, sport) =
            parse_tcx(filename, is_fit_file).expect("Failed to parse tcx");
        let (lap_list, sport) = apply_lap_corrections(lap_list, sport, corr_map);
        let first_lap = lap_list.get(0).expect("No laps found");
        GarminParseTcx {
            gfile: GarminFile {
                filename: file_name,
                filetype: "tcx".to_string(),
                begin_datetime: first_lap.lap_start.clone(),
                sport: sport,
                total_calories: lap_list.iter().map(|lap| lap.lap_calories).sum(),
                total_distance: lap_list.iter().map(|lap| lap.lap_distance).sum(),
                total_duration: lap_list.iter().map(|lap| lap.lap_duration).sum(),
                total_hr_dur: lap_list
                    .iter()
                    .map(|lap| lap.lap_avg_hr.unwrap_or(0.0) * lap.lap_duration)
                    .sum(),
                total_hr_dis: lap_list.iter().map(|lap| lap.lap_duration).sum(),
                laps: lap_list,
                points: point_list,
            },
        }
    }
}

fn parse_tcx(
    filename: &str,
    is_fit_file: bool,
) -> Result<(Vec<GarminLap>, Vec<GarminPoint>, Option<String>), Error> {
    let sport_type_map = get_sport_type_map();

    let command = if is_fit_file {
        format!("fit2tcx -i {} | xml2", filename)
    } else {
        format!("cat {} | xml2", filename)
    };

    let stream = Exec::shell(command).stream_stdout()?;

    let reader = BufReader::new(stream);

    let mut current_point = GarminPoint::new();
    let mut current_lap = GarminLap::new();

    let mut lap_list = Vec::new();
    let mut point_list = Vec::new();
    let mut sport: Option<String> = None;

    reader
        .lines()
        .filter_map(|line| match line {
            Ok(l) => {
                let entries: Vec<_> = l.split("/").collect();
                match entries.get(4) {
                    Some(&"Lap") => match entries.get(5) {
                        Some(&"Track") => match entries.get(6) {
                            Some(&"Trackpoint") => match entries.get(7) {
                                Some(_) => {
                                    current_point.read_point_tcx(&entries[7..entries.len()]);
                                }
                                None => {
                                    point_list.push(current_point.clone());
                                    current_point.clear();
                                }
                            },
                            _ => (),
                        },
                        Some(&entry) => {
                            if entry.contains("StartTime") {
                                current_lap.clear();
                            }
                            current_lap.read_lap_tcx(&entries[5..entries.len()]);
                        }
                        None => {
                            lap_list.push(current_lap.clone());
                        }
                    },
                    Some(&entry) => {
                        if entry.contains("Sport") {
                            sport = match entry.split("=").last() {
                                Some(val) => {
                                    let v = val.to_lowercase();
                                    match sport_type_map.contains_key(&v) {
                                        true => Some(v.to_string()),
                                        false => {
                                            println!("Non matching sport {}", val);
                                            None
                                        }
                                    }
                                }
                                None => None,
                            };
                        }
                    }
                    None => (),
                }
                Some("")
            }
            Err(_) => None,
        })
        .for_each(drop);
    lap_list.push(current_lap.clone());
    point_list.push(current_point.clone());

    let mut time_from_begin = 0.0;

    let point_list = point_list
        .iter()
        .enumerate()
        .filter_map(|(i, point)| {
            let mut new_point = point.clone();
            new_point.duration_from_last = match i {
                0 => 0.0,
                _ => {
                    let cur_time: DateTime<Utc> = DateTime::from_str(&new_point.time).unwrap();
                    let last_time: DateTime<Utc> =
                        DateTime::from_str(&point_list.get(i - 1).unwrap().time).unwrap();
                    (cur_time - last_time).num_seconds() as f64
                }
            };
            time_from_begin += new_point.duration_from_last;
            new_point.duration_from_begin = time_from_begin;

            match new_point.distance {
                Some(d) => {
                    if d > 0.0 {
                        Some(new_point)
                    } else {
                        None
                    }
                }
                None => None,
            }
        })
        .collect();

    let lap_list: Vec<_> = lap_list
        .into_iter()
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
        .collect();

    Ok((lap_list, point_list, sport))
}