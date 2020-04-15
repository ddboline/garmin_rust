use std::env::var;

use garmin_lib::common::garmin_config;
use garmin_lib::utils::stack_string::StackString;

#[test]
fn test_garmin_config_new() {
    let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let default_gps_dir: StackString = format!("{}/.garmin_cache/run/gps_tracks", home_dir).into();

    let gc = garmin_config::GarminConfig::new();

    assert_eq!(gc.port, 8000);
    assert_eq!(&gc.pgurl, "");
    assert_eq!(gc.gps_dir, default_gps_dir);
}

#[test]
fn test_garmin_config_get_config() {
    let test_fname = "tests/data/test.env";

    let gc = garmin_config::GarminConfig::get_config(Some(test_fname)).unwrap();

    assert_eq!(&gc.maps_api_key, "TESTKEY");
    assert_eq!(
        &gc.pgurl,
        "postgresql://test:test@localhost:5432/garmin_summary_test"
    );
    assert_eq!(&gc.gps_dir, "/tmp/gps_dir");
}
