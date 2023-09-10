use anyhow::{format_err, Error};
use std::{collections::HashMap, ffi::OsStr, path::Path};

use crate::{
    common::{
        garmin_correction_lap::GarminCorrectionLap, garmin_file::GarminFile, garmin_lap::GarminLap,
        garmin_point::GarminPoint,
    },
    utils::{date_time_wrapper::DateTimeWrapper, sport_types::SportTypes},
};

use super::{
    garmin_parse_fit::GarminParseFit, garmin_parse_gmn::GarminParseGmn,
    garmin_parse_tcx::GarminParseTcx, garmin_parse_txt::GarminParseTxt,
};

#[derive(Default, Debug)]
pub struct GarminParse {}

impl GarminParse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl GarminParseTrait for GarminParse {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        match filename.extension().and_then(OsStr::to_str) {
            Some("txt") => GarminParseTxt::new().with_file(filename, corr_map),
            Some("fit") => GarminParseFit::new().with_file(filename, corr_map),
            Some("tcx" | "TCX") => GarminParseTcx::new().with_file(filename, corr_map),
            Some("gmn") => GarminParseGmn::new().with_file(filename, corr_map),
            Some("gz") => {
                if filename.to_string_lossy().ends_with("tcx.gz") {
                    GarminParseTcx::new().with_file(filename, corr_map)
                } else {
                    Err(format_err!("Invalid extension"))
                }
            }
            _ => Err(format_err!("Invalid extension")),
        }
    }

    fn parse_file(&self, _: &Path) -> Result<ParseOutput, Error> {
        Ok(ParseOutput::default())
    }
}

#[derive(Default)]
pub struct ParseOutput {
    pub lap_list: Vec<GarminLap>,
    pub point_list: Vec<GarminPoint>,
    pub sport: SportTypes,
}

pub trait GarminParseTrait
where
    Self: Send + Sync,
{
    /// # Errors
    /// May return error if parsing and loading file fails
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error>;

    /// # Errors
    /// May return error if parsing file fails
    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error>;
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use approx::assert_abs_diff_eq;
    use std::path::Path;

    use crate::{
        common::garmin_correction_lap::GarminCorrectionLap,
        parsers::garmin_parse::{GarminParse, GarminParseTrait},
        utils::{date_time_wrapper::iso8601::convert_datetime_to_str, sport_types::SportTypes},
    };

    #[test]
    fn test_invalid_ext() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let err = GarminParse::new()
            .with_file(&Path::new("invalid.invalid"), &corr_map)
            .unwrap_err();
        assert_eq!(format!("{}", err), "Invalid extension".to_string());
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_garmin_parse_parse_gmn() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = GarminParse::new()
            .with_file(&Path::new("../tests/data/test.gmn"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename.as_str(), "test.gmn");
        assert_eq!(gfile.sport, SportTypes::Running);
        assert_eq!(gfile.filetype.as_str(), "gmn");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2011-05-07T19:43:08Z"
        );
        assert_eq!(gfile.total_calories, 122);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.points.len(), 44);
        assert_abs_diff_eq!(gfile.total_distance, 1696.85999);
        assert_abs_diff_eq!(gfile.total_duration, 280.38);
        assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 280.38);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_garmin_parse_parse_tcx() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = GarminParse::new()
            .with_file(&Path::new("../tests/data/test.tcx"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename.as_str(), "test.tcx");
        assert_eq!(gfile.sport, SportTypes::Biking);
        assert_eq!(gfile.filetype.as_str(), "tcx");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2012-11-05T11:52:21Z"
        );
        assert_eq!(gfile.total_calories, 285);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.points.len(), 182);
        assert_abs_diff_eq!(gfile.total_distance, 5981.9423828);
        assert_abs_diff_eq!(gfile.total_duration, 1037.53);
        assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_garmin_parse_parse_tcx_gz() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = GarminParse::new()
            .with_file(&Path::new("../tests/data/test.tcx.gz"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename.as_str(), "test.tcx.gz");
        assert_eq!(gfile.sport, SportTypes::Biking);
        assert_eq!(gfile.filetype.as_str(), "tcx");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2012-11-05T11:52:21Z"
        );
        assert_eq!(gfile.total_calories, 285);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.points.len(), 182);
        assert_abs_diff_eq!(gfile.total_distance, 5981.9423828);
        assert_abs_diff_eq!(gfile.total_duration, 1037.53);
        assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_garmin_parse_fit() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = GarminParse::new()
            .with_file(&Path::new("../tests/data/test.fit"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename.as_str(), "test.fit");
        assert_eq!(gfile.sport, SportTypes::Running);
        assert_eq!(gfile.filetype.as_str(), "fit");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2014-01-12T16:00:05Z"
        );
        assert_eq!(gfile.total_calories, 351);
        assert_eq!(gfile.laps.len(), 1);
        assert_eq!(gfile.points.len(), 308);
        assert_abs_diff_eq!(gfile.total_distance, 5081.34);
        assert_abs_diff_eq!(gfile.total_duration, 1451.55);
        assert_abs_diff_eq!(gfile.total_hr_dur, 220635.6);
        assert_abs_diff_eq!(gfile.total_hr_dis, 1451.55);
        Ok(())
    }
}
