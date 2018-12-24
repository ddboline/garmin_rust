#[cfg(test)]
mod tests {
    use garmin_rust::common::garmin_summary;

    #[test]
    fn test_garmin_file_test_display() {
        let garmin_summary = garmin_summary::GarminSummary {
            filename: "test_file".to_string(),
            begin_datetime: "2011-05-07T15:43:07-04:00".to_string(),
            sport: "running".to_string(),
            total_calories: 15,
            total_distance: 32.0,
            total_duration: 16.0,
            total_hr_dur: 1234.0,
            total_hr_dis: 23456.0,
            number_of_items: 5,
            md5sum: "asjgpqowiqwe".to_string(),
        };
        assert_eq!(format!("{}", garmin_summary), "GarminSummaryTable<filename=test_file,begin_datetime=2011-05-07T15:43:07-04:00,sport=running,total_calories=15,total_distance=32,total_duration=16,total_hr_dur=1234,total_hr_dis=23456,number_of_items=5,md5sum=asjgpqowiqwe>");
    }
}
