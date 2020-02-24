use anyhow::Error;
use approx::assert_abs_diff_eq;

use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::parsers::garmin_parse::GarminParseTrait;
use garmin_lib::parsers::garmin_parse_gmn;
use garmin_lib::utils::iso_8601_datetime::convert_datetime_to_str;
use garmin_lib::utils::sport_types::SportTypes;

#[test]
#[ignore]
fn test_garmin_parse_gmn() -> Result<(), Error> {
    let pool = PgPool::default();
    let corr_list =
        GarminCorrectionList::corr_list_from_json(&pool, "tests/data/garmin_corrections.json")
            .unwrap();
    let corr_map = corr_list.get_corr_list_map();
    let gfile = garmin_parse_gmn::GarminParseGmn::new()
        .with_file("tests/data/test.gmn", &corr_map)
        .unwrap();
    assert_eq!(gfile.filename, "test.gmn");
    assert_eq!(gfile.sport, SportTypes::Running);
    assert_eq!(gfile.filetype, "gmn");
    assert_eq!(
        convert_datetime_to_str(gfile.begin_datetime),
        "2011-05-07T19:43:08Z"
    );
    assert_eq!(gfile.total_calories, 122);
    assert_eq!(gfile.laps.len(), 1);
    assert_eq!(gfile.points.len(), 44);
    assert_abs_diff_eq!(gfile.total_distance, 1696.85999);
    assert_abs_diff_eq!(gfile.total_duration, 280.38);
    assert_abs_diff_eq!(gfile.total_hr_dur, 0.0);
    assert_abs_diff_eq!(gfile.total_hr_dis, 280.38);
    Ok(())
}
