#![allow(clippy::needless_pass_by_value)]

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, server, App};
use chrono::Duration;
use std::env;

use super::logged_user::AuthorizedUsers;
use crate::common::garmin_config::GarminConfig;
use crate::common::pgpool::PgPool;
use crate::http::garmin_rust_routes::{
    garmin, garmin_get_hr_data, garmin_get_hr_pace, garmin_list_gps_tracks,
};

pub struct AppState {
    pub db: Addr<PgPool>,
    pub user_list: AuthorizedUsers,
}

pub fn start_app() {
    let config = GarminConfig::get_config(None);
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());
    let nconn = config.n_db_workers;
    let pool = PgPool::new(&config.pgurl);
    let user_list = AuthorizedUsers::new();

    let addr: Addr<PgPool> = SyncArbiter::start(nconn, move || pool.clone());

    server::new(move || {
        App::with_state(AppState {
            db: addr.clone(),
            user_list: user_list.clone(),
        })
        .middleware(IdentityService::new(
            CookieIdentityPolicy::new(secret.as_bytes())
                .name("auth")
                .path("/")
                .domain(domain.as_str())
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
