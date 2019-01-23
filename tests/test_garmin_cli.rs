use garmin_rust::common::garmin_cli;

#[test]
fn test_garmin_cli_new() {
    let gcli = garmin_cli::GarminCli::new();
    assert_eq!(gcli.do_sync, false);
    assert_eq!(gcli.do_all, false);
    assert_eq!(gcli.do_bootstrap, false);
    assert_eq!(gcli.filenames, None);
}

#[test]
fn test_garmin_file_test_filenames() {
    let test_config = "tests/data/test.env";

    let gcli = garmin_cli::GarminCli {
        config: GarminConfig::get_config(Some(test_config)),
        do_sync: false,
        do_all: false,
        do_bootstrap: false,
        filenames: Some(vec![
            "tests/data/test.fit".to_string(),
            "tests/data/test.gmn".to_string(),
            "tests/data/test.tcx".to_string(),
            "tests/data/test.txt".to_string(),
        ]),
    };
}
