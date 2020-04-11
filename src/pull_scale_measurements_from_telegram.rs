use anyhow::Error;

use fitbit_bot::telegram_bot::run_bot;
use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(config.pgurl.as_str());
    run_bot(config.telegram_bot_token.as_str(), pool).await
}
