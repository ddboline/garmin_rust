#[macro_use]
extern crate approx;

use garmin_lib::common::garmin_correction_lap::{GarminCorrectionList, GarminCorrectionListTrait};
use garmin_lib::parsers::garmin_parse::{GarminParse, GarminParseTrait};

#[test]
fn test_invalid_ext() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let err = GarminParse::new()
        .with_file("invalid.invalid", &corr_map)
        .unwrap_err();
    assert_eq!(format!("{}", err), "Invalid extension".to_string())
}

#[test]
fn test_garmin_parse_parse_gmn() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = GarminParse::new()
        .with_file("tests/data/test.gmn", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.gmn");
    assert_eq!(gfile.sport.unwrap(), "running");
    assert_eq!(gfile.filetype, "gmn");
    assert_eq!(gfile.begin_datetime, "2011-05-07T19:43:08Z");
    assert_eq!(gfile.total_calories, 122);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.points.len(), 44);
    assert_abs_diff_eq!(gfile.total_distance, 1696.85999);
    assert_abs_diff_eq!(gfile.total_duration, 280.38);
    assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 280.38);
}

#[test]
fn test_garmin_parse_parse_tcx() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = GarminParse::new()
        .with_file("tests/data/test.tcx", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.tcx");
    assert_eq!(gfile.sport.unwrap(), "biking");
    assert_eq!(gfile.filetype, "tcx");
    assert_eq!(gfile.begin_datetime, "2012-11-05T11:52:21Z");
    assert_eq!(gfile.total_calories, 285);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.points.len(), 182);
    assert_abs_diff_eq!(gfile.total_distance, 5981.0424805); // 5981.9423828);
    assert_abs_diff_eq!(gfile.total_duration, 1037.53);
    assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
}

#[test]
fn test_garmin_parse_fit() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = GarminParse::new()
        .with_file("tests/data/test.fit", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.fit");
    assert_eq!(gfile.sport.unwrap(), "running");
    assert_eq!(gfile.filetype, "tcx");
    assert_eq!(gfile.begin_datetime, "2014-01-12T16:00:05Z");
    assert_eq!(gfile.total_calories, 351);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.points.len(), 308);
    assert_abs_diff_eq!(gfile.total_distance, 5081.34);
    assert_abs_diff_eq!(gfile.total_duration, 1451.55);
    assert_abs_diff_eq!(gfile.total_hr_dur, 220635.6);
    assert_abs_diff_eq!(gfile.total_hr_dis, 1451.55);
}
