#[macro_use]
extern crate approx;

use garmin_lib::common::garmin_correction_lap::{GarminCorrectionList, GarminCorrectionListTrait};
use garmin_lib::parsers::garmin_parse::GarminParseTrait;
use garmin_lib::parsers::garmin_parse_txt;

#[test]
fn test_garmin_parse_txt() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_txt::GarminParseTxt::new()
        .with_file("tests/data/test.txt", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.txt");
    assert_eq!(gfile.sport.unwrap(), "elliptical");
    assert_eq!(gfile.filetype, "txt");
    assert_eq!(gfile.begin_datetime, "2013-01-16T13:30:00Z");
    assert_eq!(gfile.total_calories, 2700);
    assert_eq!(gfile.laps.get(0).unwrap().lap_index, 0);
    assert_eq!(gfile.laps.get(1).unwrap().lap_index, 1);
    assert_eq!(gfile.laps.len(), 3);
    assert_eq!(gfile.points.len(), 3);
    assert_abs_diff_eq!(gfile.total_distance, 17702.784);
    assert_abs_diff_eq!(gfile.total_duration, 6600.0);
    assert_abs_diff_eq!(gfile.total_hr_dur, 1881000.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 6600.0);
}