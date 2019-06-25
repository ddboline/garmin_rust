#[macro_use]
extern crate approx;

use roxmltree::{Document, NodeType};
use subprocess::{Exec, Redirection};

use garmin_lib::common::garmin_correction_lap::{GarminCorrectionList, GarminCorrectionListTrait};
use garmin_lib::common::garmin_lap::GarminLap;
use garmin_lib::parsers::garmin_parse::GarminParseTrait;
use garmin_lib::parsers::garmin_parse_gmn;
use garmin_lib::utils::sport_types::SportTypes;

#[test]
fn test_garmin_parse_gmn() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_gmn::GarminParseGmn::new()
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
fn test_garmin_parse_gmn_roxmltree() {
    let filename = "tests/data/test.gmn";
    let gparse = garmin_parse_gmn::GarminParseGmn::new();
    let command = format!(
        "echo \"{}\" `garmin_dump {}` \"{}\"",
        "<root>", filename, "</root>"
    );
    let output = Exec::shell(command)
        .stdout(Redirection::Pipe)
        .capture()
        .unwrap()
        .stdout_str();
    let doc = Document::parse(&output).unwrap();

    let mut lap_list = Vec::new();
    let mut sport: Option<SportTypes> = None;

    for d in doc.root().descendants() {
        if d.node_type() == NodeType::Element && d.tag_name().name() == "run" {
            for a in d.attributes() {
                if a.name() == "sport" {
                    sport = a.value().parse().ok();
                }
            }
        }
        if d.node_type() == NodeType::Element && d.tag_name().name() == "lap" {
            lap_list.push(GarminLap::read_lap_xml_new(&d).unwrap());
        }
        if d.node_type() == NodeType::Element && d.tag_name().name() == "point" {
            
        }
    }
    println!("{:?}", lap_list);
    assert!(false);
}
