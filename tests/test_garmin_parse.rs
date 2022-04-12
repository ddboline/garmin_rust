use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::path::Path;

use garmin_lib::{
    common::garmin_correction_lap::GarminCorrectionLap,
    parsers::garmin_parse::{GarminParse, GarminParseTrait},
    utils::{date_time_wrapper::iso8601::convert_datetime_to_str, sport_types::SportTypes},
};

#[test]
fn test_invalid_ext() -> Result<(), Error> {
    let corr_map =
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
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
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = GarminParse::new()
        .with_file(&Path::new("tests/data/test.gmn"), &corr_map)
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
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = GarminParse::new()
        .with_file(&Path::new("tests/data/test.tcx"), &corr_map)
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
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = GarminParse::new()
        .with_file(&Path::new("tests/data/test.tcx.gz"), &corr_map)
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
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = GarminParse::new()
        .with_file(&Path::new("tests/data/test.fit"), &corr_map)
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
