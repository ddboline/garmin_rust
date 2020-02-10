use anyhow::Error;

use garmin_lib::common::garmin_cli::{GarminCli, GarminCliOptions};

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::parsers::garmin_parse::GarminParse;

#[test]
#[ignore]
fn test_garmin_file_test_filenames() -> Result<(), Error> {
    let test_config = "tests/data/test.env";
    let config = GarminConfig::get_config(Some(test_config))?;
    let pool = PgPool::new(&config.pgurl);
    let corr = GarminCorrectionList::new(&pool);

    let gcli = GarminCli {
        config,
        opts: Some(GarminCliOptions::FileNames(vec![
            "tests/data/test.fit".to_string(),
            "tests/data/test.gmn".to_string(),
            "tests/data/test.tcx".to_string(),
            "tests/data/test.txt".to_string(),
        ])),
        pool,
        corr,
        parser: GarminParse::new(),
    };

    assert!(gcli.opts.is_some());
    Ok(())
}
