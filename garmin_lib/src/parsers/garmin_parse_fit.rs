use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use fitparser::profile::field_types::MesgNum;
use fitparser::Value;
use log::debug;
use std::fs::File;
use std::{collections::HashMap, path::Path};

use super::garmin_parse::{GarminParseTrait, ParseOutput};
use crate::{
    common::{
        garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap},
        garmin_file::GarminFile,
        garmin_lap::GarminLap,
        garmin_point::GarminPoint,
    },
    utils::sport_types::SportTypes,
};

#[derive(Debug, Default)]
pub struct GarminParseFit {}

impl GarminParseFit {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GarminParseTrait for GarminParseFit {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let fit_output = self.parse_file(filename)?;
        let (lap_list, sport) =
            apply_lap_corrections(&fit_output.lap_list, fit_output.sport, corr_map);
        let first_lap = lap_list.get(0).ok_or_else(|| format_err!("No laps"))?;
        let filename = filename
            .file_name()
            .ok_or_else(|| format_err!("filename {:?} has no path", filename))?
            .to_string_lossy()
            .to_string()
            .into();
        let gfile = GarminFile {
            filename,
            filetype: "fit".into(),
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
            points: fit_output.point_list,
        };
        Ok(gfile)
    }

    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error> {
        let mut f = File::open(filename)?;
        let records = fitparser::from_reader(&mut f)?;

        let mut lap_list = Vec::new();
        let mut point_list: Vec<GarminPoint> = Vec::new();
        let mut sport = SportTypes::None;

        for record in records {
            match record.kind() {
                MesgNum::Record => {
                    point_list.push(GarminPoint::read_point_fit(record.fields())?);
                }
                MesgNum::Lap => {
                    lap_list.push(GarminLap::read_lap_fit(record.fields())?);
                }
                MesgNum::Session => {
                    for field in record.fields() {
                        if field.name() == "sport" {
                            if let Value::String(s) = field.value() {
                                sport = s.parse().unwrap_or(SportTypes::None);
                            }
                        }
                    }
                }
                _ => {
                    debug!("{:?}", record.kind());
                }
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
