use anyhow::Error;

use fitbit_bot::telegram_bot::TelegramBot;
use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(config.pgurl.as_str());
    if let Some(telegram_bot_token) = config.telegram_bot_token.as_ref() {
        TelegramBot::new(telegram_bot_token, &pool)
            .run_bot()
            .await?;
    }
    Ok(())
}
