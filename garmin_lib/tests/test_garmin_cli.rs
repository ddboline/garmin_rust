use garmin_lib::common::garmin_cli::{GarminCliObj, GarminCliOptions};
use garmin_lib::common::garmin_config;
use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::parsers::garmin_parse::GarminParse;

#[test]
fn test_garmin_cli_new() {
    let gcli = GarminCliObj::<GarminParse, GarminCorrectionList>::new();
    assert_eq!(gcli.opts, None);
}

#[test]
fn test_garmin_file_test_filenames() {
    let test_config = "tests/data/test.env";

    let gcli = GarminCliObj {
        config: garmin_config::GarminConfig::get_config(Some(test_config)),
        opts: Some(GarminCliOptions::FileNames(vec![
            "tests/data/test.fit".to_string(),
            "tests/data/test.gmn".to_string(),
            "tests/data/test.tcx".to_string(),
            "tests/data/test.txt".to_string(),
        ])),
        pool: None,
        corr: GarminCorrectionList::new(),
        parser: GarminParse::new(),
    };

    assert_eq!(gcli.pool, None);
}