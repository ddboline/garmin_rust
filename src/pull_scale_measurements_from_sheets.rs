use fitbit_lib::sheets_client::run_sync_sheets;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

fn main() {
    env_logger::init();
    let config = GarminConfig::get_config(None).expect("Failed to read config");
    let pool = PgPool::new(&config.pgurl);
    run_sync_sheets(&config, &pool).expect("Failed to run sheets sync");
}
