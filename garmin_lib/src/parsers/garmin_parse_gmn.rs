use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use roxmltree::{Document, NodeType};
use std::{collections::HashMap, path::Path};
use subprocess::{Exec, Redirection};

use crate::{
    common::{
        garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap},
        garmin_file::GarminFile,
        garmin_lap::GarminLap,
        garmin_point::GarminPoint,
    },
    utils::sport_types::SportTypes,
};

use super::garmin_parse::{GarminParseTrait, ParseOutput};

#[derive(Debug, Default)]
pub struct GarminParseGmn {}

impl GarminParseGmn {
    pub fn new() -> Self {
        Self {}
    }
}

impl GarminParseTrait for GarminParseGmn {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let gmn_output = self.parse_file(filename)?;
        let filename = filename
            .file_name()
            .ok_or_else(|| format_err!("filename {:?} has no path", filename))?
            .to_string_lossy()
            .to_string()
            .into();
        let (lap_list, sport) =
            apply_lap_corrections(&gmn_output.lap_list, gmn_output.sport, corr_map);
        let first_lap = lap_list.get(0).ok_or_else(|| format_err!("No laps"))?;
        let gfile = GarminFile {
            filename,
            filetype: "gmn".into(),
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

    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error> {
        let filename = filename.to_string_lossy().to_string();
        assert!(Path::new("/usr/bin/garmin_dump").exists());
        let command = format!(
            "echo \"{}\" `garmin_dump {}` \"{}\"",
            "<root>", filename, "</root>"
        );

        let output = Exec::shell(command)
            .stdout(Redirection::Pipe)
            .capture()?
            .stdout_str();
        let doc = Document::parse(&output).map_err(|e| format_err!("{}", e))?;

        let mut lap_list = Vec::new();
        let mut point_list = Vec::new();
        let mut sport = SportTypes::None;

        for d in doc.root().descendants() {
            if d.node_type() == NodeType::Element && d.tag_name().name() == "run" {
                for a in d.attributes() {
                    if a.name() == "sport" {
                        if let Ok(sp) = a.value().parse() {
                            sport = sp;
                        }
                    }
                }
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "lap" {
                lap_list.push(GarminLap::read_lap_xml(&d)?);
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "point" {
                let new_point = GarminPoint::read_point_xml(&d)?;
                if new_point.latitude.is_some()
                    && new_point.longitude.is_some()
                    && new_point.distance > Some(0.0)
                {
                    point_list.push(new_point);
                }
            }
        }

        GarminLap::fix_lap_number(&mut lap_list);
        GarminPoint::calculate_durations(&mut point_list);

        Ok(ParseOutput {
            lap_list,
            point_list,
            sport,
        })
    }
}
