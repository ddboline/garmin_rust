use failure::Error;
use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::Path;

use crate::garmin_correction_lap::GarminCorrectionLap;
use crate::garmin_file::{apply_lap_corrections, GarminFile};
use crate::garmin_lap::GarminLap;
use crate::garmin_point::GarminPoint;
use crate::utils::garmin_util::{convert_time_string, METERS_PER_MILE};
use crate::utils::sport_types::get_sport_type_map;

#[derive(Debug)]
pub struct GarminParseTxt {
    pub gfile: GarminFile,
}

impl GarminParseTxt {
    pub fn new(
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> GarminParseTxt {
        let file_name = Path::new(&filename)
            .file_name()
            .expect(&format!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or(filename.to_string());
        let (lap_list, point_list) =
            GarminParseTxt::parse_txt(filename).expect("Failed to parse txt");
        let sport = lap_list.get(0).expect("No laps found").lap_type.clone();
        let (lap_list, sport) = apply_lap_corrections(lap_list, sport, corr_map);
        let first_lap = lap_list.get(0).expect("No laps found");
        GarminParseTxt {
            gfile: GarminFile {
                filename: file_name,
                filetype: "txt".to_string(),
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

    fn parse_txt(filename: &str) -> Result<(Vec<GarminLap>, Vec<GarminPoint>), Error> {
        let file = File::open(filename)?;
        let reader = BufReader::new(file);

        let lap_list: Vec<_> = reader
            .lines()
            .filter_map(|line| match line {
                Ok(l) => match GarminParseTxt::parse_line(&l) {
                    Ok(pl) => Some(pl),
                    Err(_) => None,
                },
                Err(_) => None,
            })
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

        let mut time_since_begin = 0.0;
        let mut distance_since_begin = 0.0;
        let duration_speed_vec: Vec<_> = lap_list
            .iter()
            .map(|lap| {
                let duration_from_last = lap.lap_duration;
                let distance_from_last = lap.lap_distance;
                time_since_begin += duration_from_last;
                distance_since_begin += distance_from_last;

                let speed_permi = if distance_from_last > 0.0 {
                    (duration_from_last / 60.) / (distance_from_last / METERS_PER_MILE)
                } else {
                    0.0
                };
                let speed_mph = if duration_from_last > 0.0 {
                    (distance_from_last / METERS_PER_MILE) / (duration_from_last / 3600.)
                } else {
                    0.0
                };
                let speed_mps = if duration_from_last > 0.0 {
                    distance_from_last / duration_from_last
                } else {
                    0.0
                };
                let avg_speed_value_permi = if distance_since_begin > 0.0 {
                    (time_since_begin / 60.) / (distance_since_begin / METERS_PER_MILE)
                } else {
                    0.0
                };
                let avg_speed_value_mph = if time_since_begin > 0.0 {
                    (distance_since_begin / METERS_PER_MILE) / (time_since_begin / 3600.)
                } else {
                    0.0
                };

                (
                    duration_from_last,
                    time_since_begin,
                    speed_permi,
                    speed_mph,
                    speed_mps,
                    avg_speed_value_permi,
                    avg_speed_value_mph,
                )
            })
            .collect();

        let point_list: Vec<_> = lap_list
            .iter()
            .zip(duration_speed_vec.iter())
            .map(
                |(
                    lap,
                    (
                        duration_from_last,
                        time_since_begin,
                        speed_permi,
                        speed_mph,
                        speed_mps,
                        avg_speed_value_permi,
                        avg_speed_value_mph,
                    ),
                )| GarminPoint {
                    time: lap.lap_start.clone(),
                    latitude: None,
                    longitude: None,
                    altitude: None,
                    distance: Some(lap.lap_distance),
                    heart_rate: None,
                    duration_from_last: *duration_from_last,
                    duration_from_begin: *time_since_begin,
                    speed_mps: *speed_mps,
                    speed_permi: *speed_permi,
                    speed_mph: *speed_mph,
                    avg_speed_value_permi: *avg_speed_value_permi,
                    avg_speed_value_mph: *avg_speed_value_mph,
                },
            )
            .collect();

        Ok((lap_list, point_list))
    }

    fn parse_line(line: &str) -> Result<GarminLap, Error> {
        let sport_type_map = get_sport_type_map();

        let entry_dict: HashMap<_, _> = line
            .split_whitespace()
            .filter_map(|x| {
                let entries: Vec<_> = x.split("=").collect();
                match entries.get(0) {
                    Some(key) => match entries.get(1) {
                        Some(val) => Some((key.to_string(), val.trim().to_string())),
                        _ => None,
                    },
                    _ => None,
                }
            })
            .collect();

        let (year, month, date): (i32, i32, i32) = match entry_dict.get("date") {
            Some(val) => (val[0..4].parse()?, val[4..6].parse()?, val[6..8].parse()?),
            _ => (-1, -1, -1),
        };

        let (hour, minute, second): (i32, i32, i32) = match entry_dict.get("time") {
            Some(val) => {
                let entries: Vec<_> = val.split(":").collect();
                match entries.get(0) {
                    Some(h) => match entries.get(1) {
                        Some(m) => match entries.get(2) {
                            Some(s) => (h.parse()?, m.parse()?, s.parse()?),
                            None => (h.parse()?, m.parse()?, 0),
                        },
                        None => (h.parse()?, 0, 0),
                    },
                    None => (0, 0, 0),
                }
            }
            None => (0, 0, 0),
        };

        let lap_start = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, date, hour, minute, second
        );

        let lap_type = match entry_dict.get("type") {
            Some(val) => match sport_type_map.get(val) {
                Some(_) => Some(val.to_string()),
                None => None,
            },
            None => None,
        };

        let lap_number: i32 = match entry_dict.get("lap") {
            Some(v) => v.parse()?,
            None => -1,
        };

        let lap_dur: f64 = match entry_dict.get("dur") {
            Some(v) => convert_time_string(v)?,
            None => 0.,
        };

        let lap_dis: f64 = match entry_dict.get("dis") {
            Some(v) => {
                if v.contains("mi") {
                    let mut tmpstr = v.to_string();
                    let at = tmpstr.len() - 2;
                    let _ = tmpstr.split_off(at);
                    let dis: f64 = tmpstr.parse()?;
                    dis * METERS_PER_MILE
                } else if v.contains("m") {
                    let mut tmpstr = v.to_string();
                    let at = tmpstr.len() - 1;
                    let _ = tmpstr.split_off(at);
                    tmpstr.parse()?
                } else {
                    v.parse()?
                }
            }
            None => 0.,
        };

        let lap_cal: i32 = match entry_dict.get("cal") {
            Some(v) => v.parse()?,
            None => 0,
        };

        let lap_avghr: Option<f64> = match entry_dict.get("avghr") {
            Some(v) => Some(v.parse()?),
            None => None,
        };

        Ok(GarminLap {
            lap_type: lap_type,
            lap_index: -1,
            lap_start: lap_start,
            lap_duration: lap_dur,
            lap_distance: lap_dis,
            lap_trigger: None,
            lap_max_speed: None,
            lap_calories: lap_cal,
            lap_avg_hr: lap_avghr,
            lap_max_hr: None,
            lap_intensity: None,
            lap_number: lap_number,
            lap_start_string: None,
        })
    }
}