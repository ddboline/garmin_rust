use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::{
    io::{stdout, Write},
    path::Path,
};

use garmin_lib::{
    common::{garmin_correction_lap::GarminCorrectionLap, garmin_file},
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_fit},
};

#[test]
#[ignore]
fn test_garmin_file_test_avro() -> Result<(), Error> {
    let corr_map = GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json")?;
    let gfile = garmin_parse_fit::GarminParseFit::new()
        .with_file(Path::new("tests/data/test.fit"), &corr_map)?;
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
