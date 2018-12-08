#[macro_use]
extern crate approx;

#[cfg(test)]
mod tests {
    #[test]
    fn test_garmin_file_test_avro() {
        let corr_list = garmin_rust::garmin_correction_lap::corr_list_from_json(
            "tests/data/garmin_corrections.json",
        )
        .unwrap();
        let corr_map = garmin_rust::garmin_correction_lap::get_corr_list_map(&corr_list);
        let gparse = garmin_rust::garmin_parse_tcx::GarminParseTcx::new(
            "tests/data/test.fit",
            &corr_map,
            true,
        );
        match gparse.gfile.dump_avro("temp.avro.gz") {
            Ok(()) => {
                println!("Success");
            }
            Err(e) => {
                println!("{}", e);
            }
        }

        match garmin_rust::garmin_file::GarminFile::read_avro("temp.avro.gz") {
            Ok(g) => {
                println!("Success");
                assert_eq!(gparse.gfile.sport, g.sport);
                assert_eq!(gparse.gfile.filename, g.filename);
                assert_eq!(gparse.gfile.sport.unwrap(), g.sport.unwrap());
                assert_eq!(gparse.gfile.filetype, g.filetype);
                assert_eq!(gparse.gfile.begin_datetime, g.begin_datetime);
                assert_eq!(gparse.gfile.total_calories, g.total_calories);
                assert_eq!(gparse.gfile.laps.len(), g.laps.len());
                assert_eq!(gparse.gfile.points.len(), g.points.len());
                assert_abs_diff_eq!(gparse.gfile.total_distance, g.total_distance);
                assert_abs_diff_eq!(gparse.gfile.total_duration, g.total_duration);
                assert_abs_diff_eq!(gparse.gfile.total_hr_dur, g.total_hr_dur);
                assert_abs_diff_eq!(gparse.gfile.total_hr_dis, g.total_hr_dis);
            }
            Err(e) => {
                println!("{}", e);
                assert!(false);
            }
        }

        std::fs::remove_file("temp.avro.gz").unwrap();
    }
}
