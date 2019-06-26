use failure::{err_msg, Error};
use roxmltree::{Document, NodeType};
use std::collections::HashMap;
use std::env::var;
use std::path::Path;
use subprocess::{Exec, Redirection};

use super::garmin_parse::{GarminParseTrait, ParseOutput};
use crate::common::garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap};
use crate::common::garmin_file::GarminFile;
use crate::common::garmin_lap::GarminLap;
use crate::common::garmin_point::GarminPoint;
use crate::utils::sport_types::SportTypes;

#[derive(Debug, Default)]
pub struct GarminParseTcx {
    pub is_fit_file: bool,
}

impl GarminParseTcx {
    pub fn new(is_fit_file: bool) -> GarminParseTcx {
        GarminParseTcx { is_fit_file }
    }
}

impl GarminParseTrait for GarminParseTcx {
    fn with_file(
        &self,
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let file_name = Path::new(&filename)
            .file_name()
            .unwrap_or_else(|| panic!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or_else(|_| filename.to_string());
        let tcx_output = self.parse_file(filename)?;
        let (lap_list, sport) =
            apply_lap_corrections(&tcx_output.lap_list, &tcx_output.sport, corr_map);
        let first_lap = lap_list.get(0).ok_or_else(|| err_msg("No laps"))?;
        let gfile = GarminFile {
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
        Ok(gfile)
    }

    fn parse_file(&self, filename: &str) -> Result<ParseOutput, Error> {
        let command = match var("LAMBDA_TASK_ROOT") {
            Ok(x) => {
                if self.is_fit_file {
                    format!("{}/bin/fit2tcx -i {}", x, filename)
                } else {
                    format!("cat {}", filename)
                }
            }
            Err(_) => {
                if self.is_fit_file {
                    format!("fit2tcx -i {}", filename)
                } else {
                    format!("cat {}", filename)
                }
            }
        };

        let output = Exec::shell(command)
            .stdout(Redirection::Pipe)
            .capture()?
            .stdout_str();
        let doc = Document::parse(&output)?;

        let mut lap_list = Vec::new();
        let mut point_list: Vec<GarminPoint> = Vec::new();
        let mut sport: Option<SportTypes> = None;

        for d in doc.root().descendants() {
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Activity" {
                for a in d.attributes() {
                    if a.name() == "Sport" {
                        sport = a.value().parse().ok();
                    }
                }
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Lap" {
                let new_lap = GarminLap::read_lap_tcx(&d)?;
                lap_list.push(new_lap);
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Trackpoint" {
                let new_point = GarminPoint::read_point_tcx(&d)?;
                if new_point.latitude.is_some() && new_point.longitude.is_some() {
                    point_list.push(new_point);
                }
            }
        }

        let point_list = GarminPoint::calculate_durations(&point_list);

        let lap_list: Vec<_> = GarminLap::fix_lap_number(lap_list);

        Ok(ParseOutput {
            lap_list,
            point_list,
            sport: sport.map(|x| x.to_string()),
        })
    }
}
