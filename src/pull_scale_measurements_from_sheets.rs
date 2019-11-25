// use fitbit_lib::sheets_client::run_sync_sheets;
use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

fn main() {
    env_logger::init();
    // run_sync_sheets().unwrap();
    let config = GarminConfig::get_config(None).unwrap();
    let pool = PgPool::new(&config.pgurl);

    FitbitHeartRate::export_db_to_avro(&config, &pool).unwrap();
}
