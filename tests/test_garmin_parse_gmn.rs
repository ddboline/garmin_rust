#[macro_use]
extern crate approx;

#[cfg(test)]
mod tests {
    #[test]
    fn test_garmin_parse_gmn() {
        let corr_list = garmin_rust::garmin_correction_lap::corr_list_from_json(
            "tests/data/garmin_corrections.json",
        ).unwrap();
        let corr_map = garmin_rust::garmin_correction_lap::get_corr_list_map(&corr_list);
        let gparse =
            garmin_rust::garmin_parse_gmn::GarminParseGmn::new("tests/data/test.gmn", &corr_map);
        assert_eq!(gparse.gfile.filename, "test.gmn");
        assert_eq!(gparse.gfile.sport.unwrap(), "running");
        assert_eq!(gparse.gfile.filetype, "gmn");
        assert_eq!(gparse.gfile.begin_datetime, "2011-05-07T19:43:08Z");
        assert_eq!(gparse.gfile.total_calories, 122);
        assert_eq!(gparse.gfile.laps.len(), 1);
        assert_eq!(gparse.gfile.points.len(), 44);
        assert_abs_diff_eq!(gparse.gfile.total_distance, 1696.85999);
        assert_abs_diff_eq!(gparse.gfile.total_duration, 280.38);
        assert_abs_diff_eq!(gparse.gfile.total_hr_dur, 0.0);
        assert_abs_diff_eq!(gparse.gfile.total_hr_dis, 280.38);
    }
}
