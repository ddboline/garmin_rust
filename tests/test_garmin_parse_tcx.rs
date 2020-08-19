use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::path::Path;

use garmin_lib::{
    common::garmin_correction_lap::GarminCorrectionLap,
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_tcx},
    utils::{iso_8601_datetime::convert_datetime_to_str, sport_types::SportTypes},
};

#[test]
#[ignore]
fn test_garmin_parse_tcx() -> Result<(), Error> {
    let corr_map =
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = garmin_parse_tcx::GarminParseTcx::new()
        .with_file(&Path::new("tests/data/test.tcx"), &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.tcx");
    assert_eq!(gfile.sport, SportTypes::Biking);
    assert_eq!(gfile.filetype, "tcx");
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
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
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = garmin_parse_tcx::GarminParseTcx::new()
        .with_file(&Path::new("tests/data/test.tcx.gz"), &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.tcx.gz");
    assert_eq!(gfile.sport, SportTypes::Biking);
    assert_eq!(gfile.filetype, "tcx");
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
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
