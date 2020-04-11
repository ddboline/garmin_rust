use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};
use sheets_lib::sheets_client::run_sync_sheets;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = GarminConfig::get_config(None).expect("Failed to read config");
    let pool = PgPool::new(config.pgurl.as_str());
    run_sync_sheets(&config, &pool)
        .await
        .expect("Failed to run sheets sync");
}
