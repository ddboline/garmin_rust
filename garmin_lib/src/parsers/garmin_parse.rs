use chrono::{DateTime, Utc};
use failure::{err_msg, Error};
use std::collections::HashMap;
use std::path::Path;

use crate::common::garmin_correction_lap::GarminCorrectionLap;
use crate::common::garmin_file::GarminFile;
use crate::common::garmin_lap::GarminLap;
use crate::common::garmin_point::GarminPoint;
use crate::parsers::garmin_parse_gmn::GarminParseGmn;
use crate::parsers::garmin_parse_tcx::GarminParseTcx;
use crate::parsers::garmin_parse_txt::GarminParseTxt;
use crate::utils::sport_types::SportTypes;

#[derive(Default, Debug)]
pub struct GarminParse {}

impl GarminParse {
    pub fn new() -> GarminParse {
        GarminParse {}
    }
}

impl GarminParseTrait for GarminParse {
    fn with_file(
        &self,
        filename: &str,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let file_path = Path::new(&filename);
        match file_path.extension() {
            Some(x) => match x.to_str() {
                Some("txt") => GarminParseTxt::new().with_file(filename, corr_map),
                Some("fit") => GarminParseTcx::new(true).with_file(filename, corr_map),
                Some("tcx") | Some("TCX") => {
                    GarminParseTcx::new(false).with_file(filename, corr_map)
                }
                Some("gmn") => GarminParseGmn::new().with_file(filename, corr_map),
                _ => Err(err_msg("Invalid extension")),
            },
            _ => Err(err_msg("No extension?")),
        }
    }

    fn parse_file(&self, _: &str) -> Result<ParseOutput, Error> {
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
    fn with_file(
        &self,
        filename: &str,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error>;

    fn parse_file(&self, filename: &str) -> Result<ParseOutput, Error>;
}
