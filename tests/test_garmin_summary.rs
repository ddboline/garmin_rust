use garmin_lib::{
    common::garmin_summary,
    utils::{iso_8601_datetime::convert_str_to_datetime, sport_types::SportTypes},
};

#[test]
fn test_garmin_file_test_display() {
    let garmin_summary = garmin_summary::GarminSummary {
        filename: "test_file".into(),
        begin_datetime: convert_str_to_datetime("2011-05-07T15:43:07-04:00").unwrap(),
        sport: SportTypes::Running,
        total_calories: 15,
        total_distance: 32.0,
        total_duration: 16.0,
        total_hr_dur: 1234.0,
        total_hr_dis: 23456.0,
        md5sum: "asjgpqowiqwe".into(),
        ..garmin_summary::GarminSummary::default()
    };
    assert_eq!(
        format!("{}", garmin_summary),
        "GarminSummaryTable<filename=test_file,begin_datetime=2011-05-07T19:43:07Z,sport=running,\
         total_calories=15,total_distance=32,total_duration=16,total_hr_dur=1234,\
         total_hr_dis=23456,md5sum=asjgpqowiqwe>"
    );
}
