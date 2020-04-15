use anyhow::Error;
use approx::assert_abs_diff_eq;

use garmin_lib::{
    common::{garmin_correction_lap::GarminCorrectionList, pgpool::PgPool},
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_tcx},
    utils::{iso_8601_datetime::convert_datetime_to_str, sport_types::SportTypes},
};

#[test]
#[ignore]
fn test_garmin_parse_tcx() -> Result<(), Error> {
    let pool = PgPool::default();

    let corr_list =
        GarminCorrectionList::corr_list_from_json(&pool, "tests/data/garmin_corrections.json")
            .unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(false)
        .with_file("tests/data/test.tcx", &corr_map)
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
fn test_garmin_parse_fit() -> Result<(), Error> {
    let pool = PgPool::default();

    let corr_list =
        GarminCorrectionList::corr_list_from_json(&pool, "tests/data/garmin_corrections.json")
            .unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(true)
        .with_file("tests/data/test.fit", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.fit");
    assert_eq!(gfile.sport, SportTypes::Running);
    assert_eq!(gfile.filetype, "tcx");
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
