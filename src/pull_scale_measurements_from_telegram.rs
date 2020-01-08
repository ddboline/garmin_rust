use anyhow::format_err;
use crossbeam_utils::thread;

use fitbit_bot::telegram_bot::run_bot;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

fn main() {
    env_logger::init();
    let config = GarminConfig::get_config(None).unwrap();
    let pool = PgPool::new(&config.pgurl);
    thread::scope(|scope| run_bot(&config.telegram_bot_token, pool, scope))
        .map_err(|x| format_err!("{:?}", x))
        .and_then(|r| r)
        .unwrap();
}
