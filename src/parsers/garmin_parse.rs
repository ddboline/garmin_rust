use failure::Error;
use std::collections::HashMap;
use std::path::Path;

use crate::common::garmin_correction_lap::GarminCorrectionLap;
use crate::common::garmin_file::GarminFile;
use crate::common::garmin_lap::GarminLap;
use crate::common::garmin_point::GarminPoint;
use crate::parsers::garmin_parse_gmn::GarminParseGmn;
use crate::parsers::garmin_parse_tcx::GarminParseTcx;
use crate::parsers::garmin_parse_txt::GarminParseTxt;

#[derive(Default)]
pub struct GarminParse {
    pub gfile: GarminFile,
    pub gtype: String,
}

impl GarminParse {
    pub fn new() -> GarminParse {
        GarminParse {
            gfile: GarminFile::new(),
            gtype: "".to_string(),
        }
    }

    pub fn with_file(
        self,
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Self {
        let file_path = Path::new(&filename);
        match file_path.extension() {
            Some(x) => match x.to_str() {
                Some("txt") => GarminParse {
                    gfile: GarminParseTxt::new().with_file(filename, corr_map).gfile,
                    gtype: "txt".to_string(),
                },
                Some("fit") => GarminParse {
                    gfile: GarminParseTcx::new(true)
                        .with_file(filename, corr_map)
                        .gfile,
                    gtype: "fit".to_string(),
                },
                Some("tcx") | Some("TCX") => GarminParse {
                    gfile: GarminParseTcx::new(false)
                        .with_file(filename, corr_map)
                        .gfile,
                    gtype: "tcx".to_string(),
                },
                Some("gmn") => GarminParse {
                    gfile: GarminParseGmn::new().with_file(filename, corr_map).gfile,
                    gtype: "gmn".to_string(),
                },
                _ => GarminParse::new(),
            },
            _ => GarminParse::new(),
        }
    }
}

#[derive(Default)]
pub struct ParseOutput {
    pub lap_list: Vec<GarminLap>,
    pub point_list: Vec<GarminPoint>,
    pub sport: Option<String>,
}

pub trait GarminParseTrait {
    fn with_file(
        self,
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Self;

    fn parse_file(&self, filename: &str) -> Result<ParseOutput, Error>;
}
