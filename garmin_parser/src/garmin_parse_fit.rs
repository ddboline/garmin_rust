use anyhow::{format_err, Error};
use fitparser::{profile::field_types::MesgNum, Value};
use log::debug;
use std::{collections::HashMap, fs::File, path::Path};

use garmin_lib::date_time_wrapper::DateTimeWrapper;
use garmin_utils::sport_types::SportTypes;

use garmin_models::{
    garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap},
    garmin_file::GarminFile,
    garmin_lap::GarminLap,
    garmin_point::GarminPoint,
};

use crate::garmin_parse::{GarminParseTrait, ParseOutput};

#[derive(Debug, Default)]
pub struct GarminParseFit {}

impl GarminParseFit {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl GarminParseTrait for GarminParseFit {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let fit_output = self.parse_file(filename)?;
        let (lap_list, sport) =
            apply_lap_corrections(&fit_output.lap_list, fit_output.sport, corr_map);
        let first_lap = lap_list.first().ok_or_else(|| format_err!("No laps"))?;
        let filename = filename
            .file_name()
            .ok_or_else(|| format_err!("filename {filename:?} has no path"))?
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
        if !filename.exists() {
            return Err(format_err!("file {filename:?} does not exist"));
        }
        let mut f = File::open(filename)?;
        let records = fitparser::from_reader(&mut f).map_err(|e| format_err!("{e:?}"))?;

        let mut lap_list = Vec::new();
        let mut point_list = Vec::new();
        let mut sport = SportTypes::None;

        for record in records {
            match record.kind() {
                MesgNum::Record => {
                    let new_point = GarminPoint::read_point_fit(record.fields());
                    if new_point.latitude.is_some()
                        && new_point.longitude.is_some()
                        && new_point.distance > Some(0.0)
                    {
                        point_list.push(new_point);
                    }
                }
                MesgNum::Lap => {
                    let (new_lap, lap_sport) = GarminLap::read_lap_fit(record.fields());
                    if let Some(sp) = lap_sport {
                        sport = sp;
                    }
                    lap_list.push(new_lap);
                }
                MesgNum::Session => {
                    for field in record.fields() {
                        if field.name() == "sport" {
                            if let Value::String(s) = field.value() {
                                if let Ok(sp) = s.parse() {
                                    sport = sp;
                                }
                            }
                        }
                    }
                }
                _ => {
                    debug!("{:?}", record.kind());
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

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use approx::assert_abs_diff_eq;
    use std::path::Path;

    use garmin_lib::date_time_wrapper::iso8601::convert_datetime_to_str;
    use garmin_models::garmin_correction_lap::GarminCorrectionLap;
    use garmin_utils::sport_types::SportTypes;

    use crate::{garmin_parse::GarminParseTrait, garmin_parse_fit};

    #[test]
    #[ignore]
    fn test_garmin_parse_fit() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = garmin_parse_fit::GarminParseFit::new()
            .with_file(&Path::new("../tests/data/test.fit"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename, "test.fit");
        assert_eq!(gfile.sport, SportTypes::Running);
        assert_eq!(gfile.filetype, "fit");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2014-01-12T16:00:05Z"
        );
        assert_eq!(gfile.total_calories, 351);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.laps[0].lap_duration, 1451.55);
        assert_eq!(gfile.points.len(), 308);
        assert_abs_diff_eq!(gfile.total_distance, 5081.34);
        assert_abs_diff_eq!(gfile.total_duration, 1451.55);
        assert_abs_diff_eq!(gfile.total_hr_dur, 220635.6);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1451.55);
        Ok(())
    }
}
