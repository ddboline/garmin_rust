#![allow(clippy::needless_pass_by_value)]

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{web, App, HttpServer};
use chrono::Duration;
use futures::future::Future;
use futures::stream::Stream;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time;
use tokio_timer::Interval;

use garmin_lib::common::pgpool::PgPool;

use super::logged_user::AUTHORIZED_USERS;
use crate::garmin_rust_routes::{
    fitbit_auth, fitbit_bodyweight, fitbit_bodyweight_sync, fitbit_callback, fitbit_heartrate_api,
    fitbit_heartrate_db, fitbit_heartrate_db_update, fitbit_plots, fitbit_sync, garmin,
    garmin_connect_sync, garmin_get_hr_data, garmin_get_hr_pace, garmin_list_gps_tracks,
    garmin_sync, garmin_upload, scale_measurement, scale_measurement_update, strava_activities,
    strava_auth, strava_callback, strava_sync, strava_update, strava_upload,
};
use crate::CONFIG;

/// AppState is the application state shared between all the handlers
/// db can be used to send messages to the database workers, each running on their own thread
/// user_list contains a shared cache of previously authorized users
pub struct AppState {
    pub db: Addr<PgPool>,
    pub history: Arc<RwLock<Vec<String>>>,
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

    let _p = pool.clone();

    actix_rt::spawn(
        Interval::new(time::Instant::now(), time::Duration::from_secs(60))
            .for_each(move |_| {
                AUTHORIZED_USERS.fill_from_db(&_p).unwrap_or(());
                Ok(())
            })
            .map_err(|e| panic!("error {:?}", e)),
    );

    let addr: Addr<PgPool> = SyncArbiter::start(config.n_db_workers, move || pool.clone());
    let history = Arc::new(RwLock::new(Vec::new()));

    HttpServer::new(move || {
        App::new()
            .data(AppState {
                db: addr.clone(),
                history: history.clone(),
            })
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(config.secret_key.as_bytes())
                    .name("auth")
                    .path("/")
                    .domain(config.domain.as_str())
                    .max_age_time(Duration::days(1))
                    .secure(false), // this can only be true if you have https
            ))
            .service(web::resource("/garmin").route(web::get().to_async(garmin)))
            .service(
                web::resource("/garmin/upload_file").route(web::post().to_async(garmin_upload)),
            )
            .service(
                web::resource("/garmin/list_gps_tracks")
                    .route(web::get().to_async(garmin_list_gps_tracks)),
            )
            .service(
                web::resource("/garmin/get_hr_data").route(web::get().to_async(garmin_get_hr_data)),
            )
            .service(
                web::resource("/garmin/get_hr_pace").route(web::get().to_async(garmin_get_hr_pace)),
            )
            .service(
                web::resource("/garmin/garmin_connect_sync")
                    .route(web::get().to_async(garmin_connect_sync)),
            )
            .service(web::resource("/garmin/garmin_sync").route(web::get().to_async(garmin_sync)))
            .service(web::resource("/garmin/strava_sync").route(web::get().to_async(strava_sync)))
            .service(web::resource("/garmin/fitbit/auth").route(web::get().to_async(fitbit_auth)))
            .service(
                web::resource("/garmin/fitbit/callback")
                    .route(web::get().to_async(fitbit_callback)),
            )
            .service(
                web::resource("/garmin/fitbit/heartrate_api")
                    .route(web::get().to_async(fitbit_heartrate_api)),
            )
            .service(
                web::resource("/garmin/fitbit/heartrate_db")
                    .route(web::get().to_async(fitbit_heartrate_db))
                    .route(web::post().to_async(fitbit_heartrate_db_update)),
            )
            .service(web::resource("/garmin/fitbit/sync").route(web::get().to_async(fitbit_sync)))
            .service(
                web::resource("/garmin/fitbit/bodyweight")
                    .route(web::get().to_async(fitbit_bodyweight)),
            )
            .service(
                web::resource("/garmin/fitbit/bodyweight_sync")
                    .route(web::get().to_async(fitbit_bodyweight_sync)),
            )
            .service(web::resource("/garmin/fitbit/plots").route(web::get().to_async(fitbit_plots)))
            .service(
                web::resource("/garmin/scale_measurements")
                    .route(web::get().to_async(scale_measurement))
                    .route(web::post().to_async(scale_measurement_update)),
            )
            .service(web::resource("/garmin/strava/auth").route(web::get().to_async(strava_auth)))
            .service(
                web::resource("/garmin/strava/callback")
                    .route(web::get().to_async(strava_callback)),
            )
            .service(
                web::resource("/garmin/strava/activities")
                    .route(web::get().to_async(strava_activities)),
            )
            .service(
                web::resource("/garmin/strava/upload").route(web::post().to_async(strava_upload)),
            )
            .service(
                web::resource("/garmin/strava/update").route(web::post().to_async(strava_update)),
            )
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", config.port))
    .start();
}
