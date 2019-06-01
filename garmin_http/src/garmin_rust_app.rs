#![allow(clippy::needless_pass_by_value)]

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, server, App};
use chrono::Duration;

use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

use super::logged_user::AuthorizedUsers;
use crate::garmin_rust_routes::{
    garmin, garmin_get_hr_data, garmin_get_hr_pace, garmin_list_gps_tracks,
};

lazy_static! {
    static ref CONFIG: GarminConfig = GarminConfig::get_config(None);
}

/// AppState is the application state shared between all the handlers
/// db can be used to send messages to the database workers, each running on their own thread
/// user_list contains a shared cache of previously authorized users
pub struct AppState {
    pub db: Addr<PgPool>,
    pub user_list: AuthorizedUsers,
}

/// Create the actix-web server.
/// Configuration is done through environment variables, see GarminConfig for more information.
/// SyncArbiter spins up config.n_db_workers workers, each on their own thread,
/// PgPool is a wrapper around a connection pool.
/// Addr is a handle through which a message can be sent to an actor.
/// We create several routes:
///    /garmin is the main route, providing the same functionality as the CLI interface, while adding the ability of upload to strava.
///    /garmin/list_gps_tracks, /garmin/get_hr_data and /garmin/get_hr_pace return structured json intended for separate analysis
pub fn start_app() {
    let config = &CONFIG;
    let pool = PgPool::new(&config.pgurl);

    let addr: Addr<PgPool> = SyncArbiter::start(config.n_db_workers, move || pool.clone());

    server::new(move || {
        App::with_state(AppState {
            db: addr.clone(),
            user_list: AuthorizedUsers::new(),
        })
        .middleware(IdentityService::new(
            CookieIdentityPolicy::new(config.secret_key.as_bytes())
                .name("auth")
                .path("/")
                .domain(config.domain.as_str())
                .max_age(Duration::days(1))
                .secure(false), // this can only be true if you have https
        ))
        .resource("/garmin", |r| r.method(Method::GET).with(garmin))
        .resource("/garmin/list_gps_tracks", |r| {
            r.method(Method::GET).with(garmin_list_gps_tracks)
        })
        .resource("/garmin/get_hr_data", |r| {
            r.method(Method::GET).with(garmin_get_hr_data)
        })
        .resource("/garmin/get_hr_pace", |r| {
            r.method(Method::GET).with(garmin_get_hr_pace)
        })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", config.port))
    .start();
}
