#[macro_use]
extern crate approx;

#[cfg(test)]
mod tests {
    use garmin_rust::garmin_correction_lap;
    use garmin_rust::parsers::garmin_parse_txt;

    #[test]
    fn test_garmin_parse_txt() {
        let corr_list =
            garmin_correction_lap::corr_list_from_json("tests/data/garmin_corrections.json")
                .unwrap();
        let corr_map = garmin_correction_lap::get_corr_list_map(&corr_list);
        let txt_file = garmin_parse_txt::GarminParseTxt::new("tests/data/test.txt", &corr_map);
        assert_eq!(txt_file.gfile.filename, "test.txt");
        assert_eq!(txt_file.gfile.sport.unwrap(), "elliptical");
        assert_eq!(txt_file.gfile.filetype, "txt");
        assert_eq!(txt_file.gfile.begin_datetime, "2013-01-16T13:30:00Z");
        assert_eq!(txt_file.gfile.total_calories, 2700);
        assert_eq!(txt_file.gfile.laps.get(0).unwrap().lap_index, 0);
        assert_eq!(txt_file.gfile.laps.get(1).unwrap().lap_index, 1);
        assert_eq!(txt_file.gfile.laps.len(), 3);
        assert_eq!(txt_file.gfile.points.len(), 3);
        assert_abs_diff_eq!(txt_file.gfile.total_distance, 17702.784);
        assert_abs_diff_eq!(txt_file.gfile.total_duration, 6600.0);
        assert_abs_diff_eq!(txt_file.gfile.total_hr_dur, 1881000.0);
        assert_abs_diff_eq!(txt_file.gfile.total_hr_dis, 6600.0);
    }
}
