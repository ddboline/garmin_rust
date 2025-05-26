use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use stack_string::format_sstr;
use std::{collections::HashMap, ffi::OsStr, path::Path};

use garmin_lib::{date_time_wrapper::DateTimeWrapper, errors::GarminError as Error};
use garmin_models::{
    garmin_correction_lap::{CorrectionKey, GarminCorrectionLap},
    garmin_file::GarminFile,
    garmin_lap::GarminLap,
    garmin_point::GarminPoint,
    garmin_summary::GarminSummary,
};
use garmin_utils::{
    garmin_util::{get_file_list, get_md5sum},
    sport_types::SportTypes,
};

use super::{
    garmin_parse_fit::GarminParseFit, garmin_parse_gmn::GarminParseGmn,
    garmin_parse_tcx::GarminParseTcx, garmin_parse_txt::GarminParseTxt,
};

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
        corr_map: &HashMap<CorrectionKey, GarminCorrectionLap>,
    ) -> Result<GarminFile, Error>;

    /// # Errors
    /// May return error if parsing file fails
    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error>;
}

#[derive(Default, Debug)]
pub struct GarminParse {}

impl GarminParse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    /// Return error if parsing or dumping avro fails
    pub fn process_single_gps_file(
        filepath: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<CorrectionKey, GarminCorrectionLap>,
    ) -> Result<GarminSummary, Error> {
        let filename = filepath
            .file_name()
            .ok_or_else(|| {
                Error::CustomError(format_sstr!("Failed to split filename {filepath:?}"))
            })?
            .to_string_lossy();
        let cache_file = cache_dir.join(format_sstr!("{filename}.avro"));

        debug!("Get md5sum {filename} ",);
        let md5sum = get_md5sum(filepath)?;

        debug!("{filename} Found md5sum {md5sum} ",);
        let gfile = GarminParse::new().with_file(filepath, corr_map)?;
        let filename = &gfile.filename;
        match gfile.laps.first() {
            Some(l) if l.lap_start == DateTimeWrapper::sentinel_datetime() => {
                return Err(Error::CustomError(format_sstr!(
                    "{filename} has empty lap start?"
                )));
            }
            Some(_) => (),
            None => return Err(Error::CustomError(format_sstr!("{filename} has no laps?"))),
        }
        gfile.dump_avro(&cache_file)?;
        debug!("{} Found md5sum {md5sum} success", filepath.display());
        Ok(GarminSummary::new(&gfile, &md5sum))
    }

    /// # Errors
    /// Return error if parsing or dumping avro fails
    pub fn process_all_gps_files(
        gps_dir: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<CorrectionKey, GarminCorrectionLap>,
    ) -> Result<Vec<GarminSummary>, Error> {
        let path = Path::new(gps_dir);

        let mut results = get_file_list(path)
            .into_par_iter()
            .map(|input_file| {
                debug!("Process {}", input_file.display());
                let filename = input_file
                    .file_name()
                    .ok_or_else(|| {
                        Error::CustomError(format_sstr!(
                            "Failed to split input_file {input_file:?}"
                        ))
                    })?
                    .to_string_lossy();
                let cache_file = cache_dir.join(format_sstr!("{filename}.avro"));
                let md5sum = get_md5sum(&input_file)?;
                let gfile = GarminParse::new().with_file(&input_file, corr_map)?;
                let filename = &gfile.filename;
                match gfile.laps.first() {
                    Some(l) if l.lap_start == DateTimeWrapper::sentinel_datetime() => {
                        return Err(Error::CustomError(format_sstr!(
                            "{input_file:?} {filename:?} has empty lap start?"
                        )));
                    }
                    Some(_) => (),
                    None => {
                        return Err(Error::CustomError(format_sstr!(
                            "{input_file:?} {filename:?} has no laps?"
                        )));
                    }
                }
                gfile.dump_avro(&cache_file)?;
                Ok(GarminSummary::new(&gfile, &md5sum))
            })
            .collect::<Result<Vec<GarminSummary>, Error>>()?;
        results.shrink_to_fit();
        Ok(results)
    }
}

impl GarminParseTrait for GarminParse {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<CorrectionKey, GarminCorrectionLap>,
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
                    Err(Error::StaticCustomError("Invalid extension"))
                }
            }
            _ => Err(Error::StaticCustomError("Invalid extension")),
        }
    }

    fn parse_file(&self, _: &Path) -> Result<ParseOutput, Error> {
        Ok(ParseOutput::default())
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;
    use std::{
        io::{stdout, Write},
        path::Path,
    };

    use garmin_lib::{
        date_time_wrapper::iso8601::convert_datetime_to_str, errors::GarminError as Error,
    };
    use garmin_models::{garmin_correction_lap::GarminCorrectionLap, garmin_file};
    use garmin_utils::sport_types::SportTypes;

    use crate::{
        garmin_parse::{GarminParse, GarminParseTrait},
        garmin_parse_fit,
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

    #[test]
    #[ignore]
    fn test_garmin_file_test_avro() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")?;
        let gfile = garmin_parse_fit::GarminParseFit::new()
            .with_file(Path::new("../tests/data/test.fit"), &corr_map)?;
        match gfile.dump_avro(Path::new("temp.avro")) {
            Ok(()) => {
                writeln!(stdout(), "Success")?;
            }
            Err(e) => {
                writeln!(stdout(), "{}", e)?;
            }
        }

        match garmin_file::GarminFile::read_avro(Path::new("temp.avro")) {
            Ok(g) => {
                writeln!(stdout(), "Success")?;
                assert_eq!(gfile.sport, g.sport);
                assert_eq!(gfile.filename, g.filename);
                assert_eq!(gfile.sport, g.sport);
                assert_eq!(gfile.filetype, g.filetype);
                assert_eq!(gfile.begin_datetime, g.begin_datetime);
                assert_eq!(gfile.total_calories, g.total_calories);
                assert_eq!(gfile.laps.len(), g.laps.len());
                assert_eq!(gfile.points.len(), g.points.len());
                assert_abs_diff_eq!(gfile.total_distance, g.total_distance);
                assert_abs_diff_eq!(gfile.total_duration, g.total_duration);
                assert_abs_diff_eq!(gfile.total_hr_dur, g.total_hr_dur);
                assert_abs_diff_eq!(gfile.total_hr_dis, g.total_hr_dis);
            }
            Err(e) => {
                writeln!(stdout(), "{}", e)?;
                assert!(false);
            }
        }

        std::fs::remove_file("temp.avro")?;
        Ok(())
    }
}
