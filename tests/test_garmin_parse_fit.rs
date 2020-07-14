use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::path::Path;

use garmin_lib::{
    common::garmin_correction_lap::GarminCorrectionLap,
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_fit},
    utils::{iso_8601_datetime::convert_datetime_to_str, sport_types::SportTypes},
};

#[test]
#[ignore]
fn test_garmin_parse_fit() -> Result<(), Error> {
    let corr_map =
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let gfile = garmin_parse_fit::GarminParseFit::new()
        .with_file(&Path::new("tests/data/test.fit"), &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.fit");
    assert_eq!(gfile.sport, SportTypes::Running);
    assert_eq!(gfile.filetype, "fit");
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
        "2014-01-12T16:00:05Z"
    );
    assert_eq!(gfile.total_calories, 351);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.laps[0].lap_duration, 1451.55);
    assert_eq!(gfile.points.len(), 308);
    assert_abs_diff_eq!(gfile.total_distance, 5081.34);
    assert_abs_diff_eq!(gfile.total_duration, 1451.55);
    assert_abs_diff_eq!(gfile.total_hr_dur, 220635.6);
    assert_abs_diff_eq!(gfile.total_hr_dis, 1451.55);
    Ok(())
}
