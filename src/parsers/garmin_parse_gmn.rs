extern crate subprocess;

use failure::Error;
use std::collections::HashMap;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

use crate::garmin_correction_lap::GarminCorrectionLap;
use crate::garmin_file::{apply_lap_corrections, GarminFile};
use crate::garmin_lap::GarminLap;
use crate::garmin_point::GarminPoint;
use crate::utils::sport_types::get_sport_type_map;

#[derive(Debug)]
pub struct GarminParseGmn {
    pub gfile: GarminFile,
}

impl GarminParseGmn {
    pub fn new(
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> GarminParseGmn {
        let file_name = Path::new(&filename)
            .file_name()
            .expect(&format!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or(filename.to_string());
        let (lap_list, point_list, sport) =
            GarminParseGmn::parse_xml(filename).expect("Failed to parse xml");
        let (lap_list, sport) = apply_lap_corrections(lap_list, sport, corr_map);
        let first_lap = lap_list.get(0).expect("No laps found");
        GarminParseGmn {
            gfile: GarminFile {
                filename: file_name,
                filetype: "gmn".to_string(),
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

    fn parse_xml(
        filename: &str,
    ) -> Result<(Vec<GarminLap>, Vec<GarminPoint>, Option<String>), Error> {
        let sport_type_map = get_sport_type_map();

        let stream = Exec::shell(format!(
            "echo \"{}\" `garmin_dump {}` \"{}\" | xml2",
            "<root>", filename, "</root>"
        ))
        .stream_stdout()?;

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
                    match entries.get(2) {
                        Some(&"run") => match entries.get(3) {
                            Some(&entry) => {
                                if entry.contains("@sport") {
                                    sport = match entry.split("=").last() {
                                        Some(val) => match sport_type_map.contains_key(val) {
                                            true => Some(val.to_string()),
                                            false => {
                                                println!("Non matching sport {}", val);
                                                None
                                            }
                                        },
                                        None => None,
                                    };
                                }
                            }
                            None => (),
                        },
                        Some(&"lap") => match entries.get(3) {
                            Some(_) => {
                                current_lap.read_lap_xml(&entries[3..entries.len()]);
                            }
                            None => {
                                lap_list.push(current_lap.clone());
                                current_lap.clear();
                            }
                        },
                        Some(&"point") => match entries.get(3) {
                            Some(_) => {
                                current_point.read_point_xml(&entries[3..entries.len()]);
                            }
                            None => {
                                point_list.push(current_point.clone());
                                current_point.clear();
                            }
                        },
                        _ => (),
                    }
                    Some("")
                }
                Err(_) => None,
            })
            .for_each(drop);
        lap_list.push(current_lap.clone());
        point_list.push(current_point.clone());

        let point_list = GarminPoint::calculate_durations(&point_list);

        let lap_list: Vec<_> = GarminLap::fix_lap_number(lap_list);

        Ok((lap_list, point_list, sport))
    }
}
