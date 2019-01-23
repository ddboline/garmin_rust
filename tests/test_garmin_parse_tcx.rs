#[macro_use]
extern crate approx;

use garmin_rust::common::garmin_correction_lap::GarminCorrectionList;
use garmin_rust::parsers::garmin_parse::GarminParseTrait;
use garmin_rust::parsers::garmin_parse_tcx;

#[test]
fn test_garmin_parse_tcx() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gparse =
        garmin_parse_tcx::GarminParseTcx::new(false).with_file("tests/data/test.tcx", &corr_map);
    assert_eq!(gparse.gfile.filename, "test.tcx");
    assert_eq!(gparse.gfile.sport.unwrap(), "biking");
    assert_eq!(gparse.gfile.filetype, "tcx");
    assert_eq!(gparse.gfile.begin_datetime, "2012-11-05T11:52:21Z");
    assert_eq!(gparse.gfile.total_calories, 285);
    assert_eq!(gparse.gfile.laps.len(), 1);
    assert_eq!(gparse.gfile.points.len(), 182);
    assert_abs_diff_eq!(gparse.gfile.total_distance, 5981.9423828);
    assert_abs_diff_eq!(gparse.gfile.total_duration, 1037.53);
    assert_abs_diff_eq!(gparse.gfile.total_hr_dur, 0.0);
    assert_abs_diff_eq!(gparse.gfile.total_hr_dis, 1037.53);
}

#[test]
fn test_garmin_parse_fit() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gparse =
        garmin_parse_tcx::GarminParseTcx::new(true).with_file("tests/data/test.fit", &corr_map);
    assert_eq!(gparse.gfile.filename, "test.fit");
    assert_eq!(gparse.gfile.sport.unwrap(), "running");
    assert_eq!(gparse.gfile.filetype, "tcx");
    assert_eq!(gparse.gfile.begin_datetime, "2014-01-12T16:00:05Z");
    assert_eq!(gparse.gfile.total_calories, 351);
    assert_eq!(gparse.gfile.laps.len(), 1);
    assert_eq!(gparse.gfile.points.len(), 308);
    assert_abs_diff_eq!(gparse.gfile.total_distance, 5081.34);
    assert_abs_diff_eq!(gparse.gfile.total_duration, 1451.55);
    assert_abs_diff_eq!(gparse.gfile.total_hr_dur, 220635.6);
    assert_abs_diff_eq!(gparse.gfile.total_hr_dis, 1451.55);
}
