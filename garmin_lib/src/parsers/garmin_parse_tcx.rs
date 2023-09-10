use anyhow::{format_err, Error};
use flate2::read::GzDecoder;
use roxmltree::{Document, NodeType};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{read_to_string, File},
    io::Read,
    path::Path,
};

use crate::{
    common::{
        garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap},
        garmin_file::GarminFile,
        garmin_lap::GarminLap,
        garmin_point::GarminPoint,
    },
    utils::{date_time_wrapper::DateTimeWrapper, sport_types::SportTypes},
};

use super::garmin_parse::{GarminParseTrait, ParseOutput};

#[derive(Debug, Default)]
pub struct GarminParseTcx {
    pub is_gzip: bool,
}

impl GarminParseTcx {
    #[must_use]
    pub fn new() -> Self {
        Self { is_gzip: false }
    }
}

impl GarminParseTrait for GarminParseTcx {
    fn with_file(
        mut self,
        filename: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        self.is_gzip = filename.extension().and_then(OsStr::to_str) == Some("gz");
        let tcx_output = self.parse_file(filename)?;
        let (lap_list, sport) =
            apply_lap_corrections(&tcx_output.lap_list, tcx_output.sport, corr_map);
        let first_lap = lap_list.get(0).ok_or_else(|| format_err!("No laps"))?;
        let filename = filename
            .file_name()
            .ok_or_else(|| format_err!("filename {filename:?} has no path"))?
            .to_string_lossy()
            .to_string()
            .into();
        let gfile = GarminFile {
            filename,
            filetype: "tcx".into(),
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
            points: tcx_output.point_list,
        };
        Ok(gfile)
    }

    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error> {
        if !filename.exists() {
            return Err(format_err!("file {filename:?} does not exist"));
        }
        let output = if self.is_gzip {
            let mut buf = String::new();
            GzDecoder::new(File::open(filename)?).read_to_string(&mut buf)?;
            buf
        } else {
            read_to_string(filename)?
        };
        let doc = Document::parse(&output).map_err(|e| format_err!("{e}"))?;

        let mut lap_list = Vec::new();
        let mut point_list = Vec::new();
        let mut sport = SportTypes::None;

        for d in doc.root().descendants() {
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Activity" {
                for a in d.attributes() {
                    if a.name() == "Sport" {
                        sport = a.value().parse().unwrap_or(SportTypes::None);
                    }
                }
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Lap" {
                let new_lap = GarminLap::read_lap_tcx(&d)?;
                lap_list.push(new_lap);
            }
            if d.node_type() == NodeType::Element && d.tag_name().name() == "Trackpoint" {
                let new_point = GarminPoint::read_point_tcx(&d)?;
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

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use approx::assert_abs_diff_eq;
    use std::path::Path;

    use crate::{
        common::garmin_correction_lap::GarminCorrectionLap,
        parsers::{garmin_parse::GarminParseTrait, garmin_parse_tcx},
        utils::{date_time_wrapper::iso8601::convert_datetime_to_str, sport_types::SportTypes},
    };

    #[test]
    #[ignore]
    fn test_garmin_parse_tcx() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = garmin_parse_tcx::GarminParseTcx::new()
            .with_file(&Path::new("../tests/data/test.tcx"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename, "test.tcx");
        assert_eq!(gfile.sport, SportTypes::Biking);
        assert_eq!(gfile.filetype, "tcx");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2012-11-05T11:52:21Z"
        );
        assert_eq!(gfile.total_calories, 285);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.laps[0].lap_duration, 1037.53);
        assert_eq!(gfile.points.len(), 182);
        assert_abs_diff_eq!(gfile.total_distance, 5981.9423828);
        assert_abs_diff_eq!(gfile.total_duration, 1037.53);
        assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_garmin_parse_tcx_gz() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = garmin_parse_tcx::GarminParseTcx::new()
            .with_file(&Path::new("../tests/data/test.tcx.gz"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename, "test.tcx.gz");
        assert_eq!(gfile.sport, SportTypes::Biking);
        assert_eq!(gfile.filetype, "tcx");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2012-11-05T11:52:21Z"
        );
        assert_eq!(gfile.total_calories, 285);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.laps[0].lap_duration, 1037.53);
        assert_eq!(gfile.points.len(), 182);
        assert_abs_diff_eq!(gfile.total_distance, 5981.9423828);
        assert_abs_diff_eq!(gfile.total_duration, 1037.53);
        assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
        Ok(())
    }
}
