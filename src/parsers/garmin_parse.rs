use std::collections::HashMap;
use std::path::Path;

use crate::garmin_correction_lap::GarminCorrectionLap;
use crate::garmin_file::GarminFile;
use crate::parsers::garmin_parse_gmn::GarminParseGmn;
use crate::parsers::garmin_parse_tcx::GarminParseTcx;
use crate::parsers::garmin_parse_txt::GarminParseTxt;

pub struct GarminParse {
    pub gfile: GarminFile,
}

impl GarminParse {
    pub fn new(
        filename: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> GarminParse {
        let null_gfile = GarminFile {
            filename: "".to_string(),
            filetype: "".to_string(),
            begin_datetime: "".to_string(),
            sport: None,
            total_calories: -1,
            total_distance: 0.0,
            total_duration: 0.0,
            total_hr_dur: 0.0,
            total_hr_dis: 0.0,
            laps: Vec::new(),
            points: Vec::new(),
        };

        let file_path = Path::new(&filename);
        let gfile = match file_path.extension() {
            Some(x) => match x.to_str() {
                Some("txt") => GarminParseTxt::new(filename, corr_map).gfile,
                Some("fit") => GarminParseTcx::new(filename, corr_map, true).gfile,
                Some("tcx") | Some("TCX") => GarminParseTcx::new(filename, corr_map, false).gfile,
                Some("gmn") => GarminParseGmn::new(filename, corr_map).gfile,
                _ => null_gfile,
            },
            _ => null_gfile,
        };
        GarminParse { gfile }
    }
}
