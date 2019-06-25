#[macro_use]
extern crate approx;

use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::garmin_file;
use garmin_lib::parsers::garmin_parse::GarminParseTrait;
use garmin_lib::parsers::garmin_parse_tcx;

#[test]
fn test_garmin_file_test_avro() {
    let corr_list =
        GarminCorrectionList::corr_list_from_json("tests/data/garmin_corrections.json").unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(true)
        .with_file("tests/data/test.fit", &corr_map)
        .unwrap();
    match gfile.dump_avro("temp.avro.gz") {
        Ok(()) => {
            println!("Success");
        }
        Err(e) => {
            println!("{}", e);
        }
    }

    match garmin_file::GarminFile::read_avro("temp.avro.gz") {
        Ok(g) => {
            println!("Success");
            assert_eq!(gfile.sport, g.sport);
            assert_eq!(gfile.filename, g.filename);
            assert_eq!(gfile.sport.unwrap(), g.sport.unwrap());
            assert_eq!(gfile.filetype, g.filetype);
            assert_eq!(gfile.begin_datetime, g.begin_datetime);
            assert_eq!(gfile.total_calories, g.total_calories);
            assert_eq!(gfile.laps.len(), g.laps.len());
            assert_eq!(gfile.points.len(), g.points.len());
            assert_abs_diff_eq!(gfile.total_distance, g.total_distance);
            assert_abs_diff_eq!(gfile.total_duration, g.total_duration);
            assert_abs_diff_eq!(gfile.total_hr_dur, g.total_hr_dur);
            assert_abs_diff_eq!(gfile.total_hr_dis, g.total_hr_dis);
        }
        Err(e) => {
            println!("{}", e);
            assert!(false);
        }
    }

    std::fs::remove_file("temp.avro.gz").unwrap();
}
