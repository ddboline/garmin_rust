use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;
use sheets_lib::sheets_client::run_sync_sheets;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = GarminConfig::get_config(None).expect("Failed to read config");
    let pool = PgPool::new();
    run_sync_sheets(&config, &pool)
        .await
        .expect("Failed to run sheets sync");
}
