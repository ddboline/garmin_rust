use anyhow::Error;
use stdout_channel::StdoutChannel;

use garmin_cli::garmin_cli::{GarminCli, GarminCliOptions};
use garmin_lib::{
    common::{
        garmin_config::GarminConfig, garmin_correction_lap::GarminCorrectionMap, pgpool::PgPool,
    },
    parsers::garmin_parse::GarminParse,
};

#[tokio::test]
#[ignore]
async fn test_garmin_file_test_filenames() -> Result<(), Error> {
    let test_config = "tests/data/test.env";
    let config = GarminConfig::get_config(Some(test_config))?;
    let pool = PgPool::new(&config.pgurl);
    let corr = GarminCorrectionMap::new();

    let gcli = GarminCli {
        config,
        opts: Some(GarminCliOptions::FileNames(vec![
            "tests/data/test.fit".into(),
            "tests/data/test.gmn".into(),
            "tests/data/test.tcx".into(),
            "tests/data/test.txt".into(),
            "tests/data/test.tcx.gz".into(),
        ])),
        pool,
        corr,
        parser: GarminParse::new(),
        stdout: StdoutChannel::new(),
    };

    assert!(gcli.opts.is_some());
    Ok(())
}
