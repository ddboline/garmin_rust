use fitbit_lib::telegram_bot::run_bot;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

fn main() {
    let config = GarminConfig::get_config(None).unwrap();
    let pool = PgPool::new(&config.pgurl);
    run_bot(&config, pool).unwrap();
}
