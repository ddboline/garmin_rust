use chrono::{DateTime, Utc};
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
pub struct GarminParseGmn {}

impl GarminParseGmn {
    pub fn new() -> GarminParseGmn {
        GarminParseGmn {}
    }
}

impl GarminParseTrait for GarminParseGmn {
    fn with_file(
        &self,
        filename: &str,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let file_name = Path::new(&filename)
            .file_name()
            .unwrap_or_else(|| panic!("filename {} has no path", filename))
            .to_os_string()
            .into_string()
            .unwrap_or_else(|_| filename.to_string());
        let gmn_output = self.parse_file(filename)?;
        let (lap_list, sport) =
            apply_lap_corrections(&gmn_output.lap_list, gmn_output.sport, corr_map);
        let first_lap = lap_list.get(0).ok_or_else(|| err_msg("No laps"))?;
        let gfile = GarminFile {
            filename: file_name,
            filetype: "gmn".to_string(),
            begin_datetime: first_lap.lap_start,
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
            points: gmn_output.point_list,
        };
        Ok(gfile)
    }

    fn parse_file(&self, filename: &str) -> Result<ParseOutput, Error> {
        let command = match var("LAMBDA_TASK_ROOT") {
            Ok(x) => format!(
                "echo \"{}\" `{}/bin/garmin_dump {}` \"{}\"",
                "<root>", x, filename, "</root>"
            ),
            Err(_) => format!(
                "echo \"{}\" `garmin_dump {}` \"{}\"",
                "<root>", filename, "</root>"
            ),
        };

        let output = Exec::shell(command)
            .stdout(Redirection::Pipe)
            .capture()?
            .stdout_str();
        let doc = Document::parse(&output)?;

        let mut lap_list = Vec::new();
        let mut point_list = Vec::new();
        let mut sport = SportTypes::None;

        for d in doc.root().descendants() {
            if d.node_type() == NodeType::Element && d.tag_name().name() == "run" {
                for a in d.attributes() {
                    if a.name() == "sport" {
                        sport = a.value().parse().unwrap_or(SportTypes::None);
                    }
                }
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "lap" {
                lap_list.push(GarminLap::read_lap_xml(&d)?);
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "point" {
                point_list.push(GarminPoint::read_point_xml(&d)?);
            }
        }

        let point_list = GarminPoint::calculate_durations(&point_list);

        let lap_list: Vec<_> = GarminLap::fix_lap_number(lap_list);

        Ok(ParseOutput {
            lap_list,
            point_list,
            sport,
        })
    }
}
