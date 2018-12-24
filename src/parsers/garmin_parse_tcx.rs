extern crate chrono;
extern crate subprocess;

use failure::Error;
use std::collections::HashMap;
use std::env::var;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

use super::garmin_parse::{GarminParseTrait, ParseOutput};
use crate::common::garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap};
use crate::common::garmin_file::GarminFile;
use crate::common::garmin_lap::GarminLap;
use crate::common::garmin_point::GarminPoint;
use crate::utils::sport_types::get_sport_type_map;

#[derive(Debug, Default)]
pub struct GarminParseTcx {
    pub gfile: GarminFile,
    pub is_fit_file: bool,
}

impl GarminParseTcx {
    pub fn new(is_fit_file: bool) -> GarminParseTcx {
        GarminParseTcx {
            gfile: GarminFile::new(),
            is_fit_file,
        }
    }
}

impl GarminParseTrait for GarminParseTcx {
    fn with_file(
        mut self,
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Self {
        let file_name = Path::new(&filename)
            .file_name()
            .unwrap_or_else(|| panic!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or_else(|_| filename.to_string());
        let tcx_output = self.parse_file(filename).expect("Failed to parse tcx");
        let (lap_list, sport) =
            apply_lap_corrections(&tcx_output.lap_list, &tcx_output.sport, corr_map);
        let first_lap = lap_list.get(0).expect("No laps found");
        self.gfile = GarminFile {
            filename: file_name,
            filetype: "tcx".to_string(),
            begin_datetime: first_lap.lap_start.clone(),
            sport,
            total_calories: lap_list.iter().map(|lap| lap.lap_calories).sum(),
            total_distance: lap_list.iter().map(|lap| lap.lap_distance).sum(),
            total_duration: lap_list.iter().map(|lap| lap.lap_duration).sum(),
            total_hr_dur: lap_list
                .iter()
                .map(|lap| lap.lap_avg_hr.unwrap_or(0.0) * lap.lap_duration)
                .sum(),
            total_hr_dis: lap_list.iter().map(|lap| lap.lap_duration).sum(),
            laps: lap_list,
            points: tcx_output.point_list,
        };
        self
    }

    fn parse_file(&self, filename: &str) -> Result<ParseOutput, Error> {
        let sport_type_map = get_sport_type_map();

        let command = match var("LAMBDA_TASK_ROOT") {
            Ok(x) => {
                if self.is_fit_file {
                    format!("{}/bin/fit2tcx -i {} | {}/bin/xml2", x, filename, x)
                } else {
                    format!("cat {} | {}/bin/xml2", filename, x)
                }
            }
            Err(_) => {
                if self.is_fit_file {
                    format!("fit2tcx -i {} | xml2", filename)
                } else {
                    format!("cat {} | xml2", filename)
                }
            }
        };

        debug!("command {}", command);

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
                    let entries: Vec<_> = l.split('/').collect();
                    match entries.get(4) {
                        Some(&"Lap") => match entries.get(5) {
                            Some(&"Track") => {
                                if let Some(&"Trackpoint") = entries.get(6) {
                                    match entries.get(7) {
                                        Some(_) => {
                                            current_point
                                                .read_point_tcx(&entries[7..entries.len()]);
                                        }
                                        None => {
                                            point_list.push(current_point.clone());
                                            current_point.clear();
                                        }
                                    }
                                }
                            }
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
                                sport = match entry.split('=').last() {
                                    Some(val) => {
                                        let v = val.to_lowercase();
                                        if sport_type_map.contains_key(&v) {
                                            Some(v.to_string())
                                        } else {
                                            println!("Non matching sport {}", val);
                                            None
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

        let point_list = GarminPoint::calculate_durations(&point_list);

        let lap_list: Vec<_> = GarminLap::fix_lap_number(lap_list);

        Ok(ParseOutput {
            lap_list,
            point_list,
            sport,
        })
    }
}
