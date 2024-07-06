#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

use anyhow::Error;

use fitbit_bot::telegram_bot::TelegramBot;
use garmin_lib::garmin_config::GarminConfig;
use garmin_utils::pgpool::PgPool;

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(config.pgurl.as_str())?;
    if let Some(telegram_bot_token) = config.telegram_bot_token.as_ref() {
        TelegramBot::new(telegram_bot_token, &pool, &config)
            .run_bot()
            .await?;
    }
    Ok(())
}
