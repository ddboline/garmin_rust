use anyhow::Error;
use approx::assert_abs_diff_eq;
use std::io::{stdout, Write};
use std::path::Path;

use garmin_lib::{
    common::{garmin_correction_lap::GarminCorrectionList, garmin_file, pgpool::PgPool},
    parsers::{garmin_parse::GarminParseTrait, garmin_parse_tcx},
};

#[test]
#[ignore]
fn test_garmin_file_test_avro() -> Result<(), Error> {
    let pool = PgPool::default();

    let corr_list =
        GarminCorrectionList::corr_list_from_json(&pool, "tests/data/garmin_corrections.json")?;
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_tcx::GarminParseTcx::new(true)
        .with_file(Path::new("tests/data/test.fit"), &corr_map)?;
    match gfile.dump_avro(Path::new("temp.avro.gz")) {
        Ok(()) => {
            writeln!(stdout(), "Success")?;
        }
        Err(e) => {
            writeln!(stdout(), "{}", e)?;
        }
    }

    match garmin_file::GarminFile::read_avro(Path::new("temp.avro.gz")) {
        Ok(g) => {
            writeln!(stdout(), "Success")?;
            assert_eq!(gfile.sport, g.sport);
            assert_eq!(gfile.filename, g.filename);
            assert_eq!(gfile.sport, g.sport);
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
            writeln!(stdout(), "{}", e)?;
            assert!(false);
        }
    }

    std::fs::remove_file("temp.avro.gz")?;
    Ok(())
}
