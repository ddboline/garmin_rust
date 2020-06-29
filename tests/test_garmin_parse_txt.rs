use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::path::Path;

use garmin_lib::{
    common::garmin_correction_lap::GarminCorrectionLap,
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_txt},
    utils::{iso_8601_datetime::convert_datetime_to_str, sport_types::SportTypes},
};

#[test]
fn test_garmin_parse_txt() -> Result<(), Error> {
    let corr_map = GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json")?;
    let gfile = garmin_parse_txt::GarminParseTxt::new()
        .with_file(&Path::new("tests/data/test.txt"), &corr_map)
        .unwrap();
    assert_eq!(gfile.filename.as_str(), "test.txt");
    assert_eq!(gfile.sport, SportTypes::Elliptical);
    assert_eq!(gfile.filetype.as_str(), "txt");
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
        "2013-01-16T13:30:00Z"
    );
    assert_eq!(gfile.total_calories, 2700);
    assert_eq!(gfile.laps.get(0).unwrap().lap_index, 0);
    assert_eq!(gfile.laps.get(1).unwrap().lap_index, 1);
    assert_eq!(gfile.laps.len(), 3);
    assert_eq!(gfile.points.len(), 3);
    assert_abs_diff_eq!(gfile.total_distance, 17702.784);
    assert_abs_diff_eq!(gfile.total_duration, 6600.0);
    assert_abs_diff_eq!(gfile.total_hr_dur, 1881000.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 6600.0);
    Ok(())
}

#[test]
fn test_garmin_parse_txt_default_time() -> Result<(), Error> {
    let corr_map =
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = garmin_parse_txt::GarminParseTxt::new()
        .with_file(&Path::new("tests/data/test2.txt"), &corr_map)
        .unwrap();
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
        "2013-01-17T12:00:00Z"
    );
    Ok(())
}
