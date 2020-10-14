use std::{env, path::Path};

use garmin_lib::common::garmin_config;

#[test]
fn test_garmin_config_new() {
    let home_dir = dirs::home_dir().unwrap();
    let default_gps_dir = home_dir
        .join(".garmin_cache")
        .join("run")
        .join("gps_tracks");

    let gc = garmin_config::GarminConfig::new();

    assert_eq!(gc.port, 8000);
    assert_eq!(&gc.pgurl, "");
    assert_eq!(gc.gps_dir, default_gps_dir);
}

#[test]
fn test_garmin_config_get_config() {
    let current_pgurl = env::var_os("PGURL");
    if current_pgurl.is_some() {
        env::remove_var("PGURL");
    }
    let test_fname = "tests/data/test.env";

    let gc = garmin_config::GarminConfig::get_config(Some(test_fname)).unwrap();

    if let Some(pgurl) = current_pgurl {
        env::set_var("PGURL", pgurl);
    }
    assert_eq!(&gc.maps_api_key, "TESTKEY");
    assert_eq!(
        &gc.pgurl,
        "postgresql://test:test@localhost:5432/garmin_summary_test"
    );
    assert_eq!(&gc.gps_dir, &Path::new("/tmp/gps_dir"));
}
