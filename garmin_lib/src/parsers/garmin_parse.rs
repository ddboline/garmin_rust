use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use std::{collections::HashMap, ffi::OsStr, path::Path};

use crate::{
    common::{
        garmin_correction_lap::GarminCorrectionLap, garmin_file::GarminFile, garmin_lap::GarminLap,
        garmin_point::GarminPoint,
    },
    utils::sport_types::SportTypes,
};

use super::{
    garmin_parse_fit::GarminParseFit, garmin_parse_gmn::GarminParseGmn,
    garmin_parse_tcx::GarminParseTcx, garmin_parse_txt::GarminParseTxt,
};

#[derive(Default, Debug)]
pub struct GarminParse {}

impl GarminParse {
    pub fn new() -> Self {
        Self {}
    }
}

impl GarminParseTrait for GarminParse {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
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
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error>;

    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error>;
}
