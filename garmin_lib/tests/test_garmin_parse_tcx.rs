#[macro_use]
extern crate approx;

use roxmltree::{Document, NodeType};
use std::fs::File;
use std::io::Read;

use garmin_lib::common::garmin_correction_lap::{GarminCorrectionList, GarminCorrectionListTrait};
use garmin_lib::parsers::garmin_parse::GarminParseTrait;
use garmin_lib::parsers::garmin_parse_tcx;
use garmin_lib::common::garmin_lap::GarminLap;
use garmin_lib::common::garmin_point::GarminPoint;
use garmin_lib::utils::sport_types::SportTypes;

#[test]
fn test_garmin_parse_tcx() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(false)
        .with_file("tests/data/test.tcx", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.tcx");
    assert_eq!(gfile.sport.unwrap(), "biking");
    assert_eq!(gfile.filetype, "tcx");
    assert_eq!(gfile.begin_datetime, "2012-11-05T11:52:21Z");
    assert_eq!(gfile.total_calories, 285);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.points.len(), 182);
    assert_abs_diff_eq!(gfile.total_distance, 5981.9423828);
    assert_abs_diff_eq!(gfile.total_duration, 1037.53);
    assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 1037.53);
}

#[test]
fn test_garmin_parse_fit() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(true)
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

#[test]
fn test_garmin_parse_tcx_roxmltree() {
    let mut f = File::open("tests/data/test.tcx").unwrap();
    let mut buf = String::new();
    f.read_to_string(&mut buf).unwrap();
    let doc = Document::parse(&buf).unwrap();

    let mut lap_list = Vec::new();
    let mut point_list = Vec::new();

    for d in doc.root().descendants() {
        if d.node_type() == NodeType::Element && d.tag_name().name() == "Activity" {
            for a in d.attributes() {
                if a.name() == "Sport" {
                    let sport: SportTypes = a.value().parse().unwrap();
                    println!("{:?}", sport);
                }
            }
        }
        if d.node_type() == NodeType::Element && d.tag_name().name() == "Lap" {
            lap_list.push(GarminLap::read_lap_tcx_new(&d).unwrap());
        }
        if d.node_type() == NodeType::Element && d.tag_name().name() == "Trackpoint" {
            let new_point = GarminPoint::read_point_tcx_new(&d).unwrap();
            println!("new_point {:?}", new_point);
            point_list.push(new_point);
        }
    }
    assert!(false);
}
