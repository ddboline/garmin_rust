#![allow(clippy::needless_pass_by_value)]

use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_session::CookieSession;
use actix_web::{web, App, HttpServer, middleware::Compress};
use anyhow::Error;
use std::time::Duration;
use tokio::time::interval;

use garmin_lib::common::pgpool::PgPool;

use crate::{
    garmin_requests::close_connect_proxy,
    garmin_rust_routes::{
        add_garmin_correction, fitbit_activities, fitbit_activities_db,
        fitbit_activities_db_update, fitbit_activity_types, fitbit_auth, fitbit_bodyweight,
        fitbit_bodyweight_sync, fitbit_callback, fitbit_heartrate_api, fitbit_heartrate_cache,
        fitbit_plots, fitbit_plots_demo, fitbit_profile, fitbit_refresh, fitbit_sync,
        fitbit_tcx_sync, garmin, garmin_connect_activities, garmin_connect_activities_db,
        garmin_connect_activities_db_update, garmin_connect_hr_api, garmin_connect_hr_sync,
        garmin_connect_sync, garmin_connect_user_summary, garmin_demo, garmin_sync, garmin_upload,
        heartrate_plots, heartrate_plots_demo, heartrate_statistics_plots,
        heartrate_statistics_plots_demo, race_result_flag, race_result_import, race_result_plot,
        race_result_plot_demo, race_results_db, race_results_db_update, scale_measurement,
        scale_measurement_update, strava_activities, strava_activities_db,
        strava_activities_db_update, strava_athlete, strava_auth, strava_callback, strava_create,
        strava_refresh, strava_sync, strava_update, strava_upload, user,
    },
    logged_user::{fill_from_db, get_secrets, SECRET_KEY, TRIGGER_DB_UPDATE},
    CONFIG,
};

/// `AppState` is the application state shared between all the handlers
/// db can be used to send messages to the database workers, each running on
/// their own thread `user_list` contains a shared cache of previously
/// authorized users
pub struct AppState {
    pub db: PgPool,
}

/// Create the actix-web server.
/// Configuration is done through environment variables, see `GarminConfig` for
/// more information. `PgPool` is a wrapper around a connection pool.
/// We create several routes:
///    `/garmin` is the main route, providing the same functionality as the CLI
/// interface, while adding the ability of upload to strava, and
/// `/garmin/get_hr_pace` return structured json intended for separate analysis
pub async fn start_app() -> Result<(), Error> {
    async fn update_db(pool: PgPool) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            close_connect_proxy().await.unwrap_or(());
            i.tick().await;
        }
    }

    TRIGGER_DB_UPDATE.set();
    get_secrets(&CONFIG.secret_path, &CONFIG.jwt_secret_path).await?;

    let pool = PgPool::new(&CONFIG.pgurl);

    actix_rt::spawn(update_db(pool.clone()));

    HttpServer::new(move || {
        App::new()
            .data(AppState { db: pool.clone() })
            .wrap(Compress::default())
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(&SECRET_KEY.get())
                    .name("auth")
                    .path("/")
                    .domain(CONFIG.domain.as_str())
                    .max_age(24 * 3600)
                    .secure(false), // this can only be true if you have https
            ))
            .wrap(
                CookieSession::private(&SECRET_KEY.get())
                    .domain(CONFIG.domain.as_str())
                    .path("/")
                    .name("session")
                    .secure(false),
            )
            .service(
                web::scope("/garmin")
                    .service(web::resource("/index.html").route(web::get().to(garmin)))
                    .service(web::resource("/demo.html").route(web::get().to(garmin_demo)))
                    .service(web::resource("/upload_file").route(web::post().to(garmin_upload)))
                    .service(
                        web::resource("/add_garmin_correction")
                            .route(web::post().to(add_garmin_correction)),
                    )
                    .service(
                        web::resource("/garmin_connect_sync")
                            .route(web::get().to(garmin_connect_sync)),
                    )
                    .service(
                        web::resource("/garmin_connect_activities")
                            .route(web::get().to(garmin_connect_activities)),
                    )
                    .service(
                        web::resource("/garmin_connect_activities_db")
                            .route(web::get().to(garmin_connect_activities_db))
                            .route(web::post().to(garmin_connect_activities_db_update)),
                    )
                    .service(
                        web::resource("/garmin_connect_hr_sync")
                            .route(web::get().to(garmin_connect_hr_sync)),
                    )
                    .service(
                        web::resource("/garmin_connect_hr_api")
                            .route(web::get().to(garmin_connect_hr_api)),
                    )
                    .service(
                        web::resource("/garmin_connect_user_summary")
                            .route(web::get().to(garmin_connect_user_summary)),
                    )
                    .service(web::resource("/garmin_sync").route(web::get().to(garmin_sync)))
                    .service(web::resource("/strava_sync").route(web::get().to(strava_sync)))
                    .service(
                        web::scope("/fitbit")
                            .service(web::resource("/auth").route(web::get().to(fitbit_auth)))
                            .service(
                                web::resource("/refresh_auth").route(web::get().to(fitbit_refresh)),
                            )
                            .service(
                                web::resource("/callback").route(web::get().to(fitbit_callback)),
                            )
                            .service(
                                web::resource("/heartrate_api")
                                    .route(web::get().to(fitbit_heartrate_api)),
                            )
                            .service(
                                web::resource("/heartrate_cache")
                                    .route(web::get().to(fitbit_heartrate_cache)),
                            )
                            .service(web::resource("/sync").route(web::get().to(fitbit_sync)))
                            .service(
                                web::resource("/bodyweight")
                                    .route(web::get().to(fitbit_bodyweight)),
                            )
                            .service(
                                web::resource("/bodyweight_sync")
                                    .route(web::get().to(fitbit_bodyweight_sync)),
                            )
                            .service(web::resource("/plots").route(web::get().to(fitbit_plots)))
                            .service(
                                web::resource("/plots_demo")
                                    .route(web::get().to(fitbit_plots_demo)),
                            )
                            .service(
                                web::resource("/heartrate_statistics_plots")
                                    .route(web::get().to(heartrate_statistics_plots)),
                            )
                            .service(
                                web::resource("/heartrate_statistics_plots_demo")
                                    .route(web::get().to(heartrate_statistics_plots_demo)),
                            )
                            .service(
                                web::resource("/heartrate_plots")
                                    .route(web::get().to(heartrate_plots)),
                            )
                            .service(
                                web::resource("/heartrate_plots_demo")
                                    .route(web::get().to(heartrate_plots_demo)),
                            )
                            .service(
                                web::resource("/fitbit_tcx_sync")
                                    .route(web::get().to(fitbit_tcx_sync)),
                            )
                            .service(
                                web::resource("/fitbit_activity_types")
                                    .route(web::get().to(fitbit_activity_types)),
                            )
                            .service(
                                web::resource("/fitbit_activities")
                                    .route(web::get().to(fitbit_activities)),
                            )
                            .service(
                                web::resource("/fitbit_activities_db")
                                    .route(web::get().to(fitbit_activities_db))
                                    .route(web::post().to(fitbit_activities_db_update)),
                            )
                            .service(
                                web::resource("/profile").route(web::get().to(fitbit_profile)),
                            ),
                    )
                    .service(
                        web::resource("/scale_measurements")
                            .route(web::get().to(scale_measurement))
                            .route(web::post().to(scale_measurement_update)),
                    )
                    .service(
                        web::scope("/strava")
                            .service(web::resource("/auth").route(web::get().to(strava_auth)))
                            .service(
                                web::resource("/refresh_auth").route(web::get().to(strava_refresh)),
                            )
                            .service(
                                web::resource("/callback").route(web::get().to(strava_callback)),
                            )
                            .service(
                                web::resource("/activities")
                                    .route(web::get().to(strava_activities)),
                            )
                            .service(web::resource("/athlete").route(web::get().to(strava_athlete)))
                            .service(
                                web::resource("/activities_db")
                                    .route(web::get().to(strava_activities_db))
                                    .route(web::post().to(strava_activities_db_update)),
                            )
                            .service(web::resource("/upload").route(web::post().to(strava_upload)))
                            .service(web::resource("/update").route(web::post().to(strava_update)))
                            .service(web::resource("/create").route(web::get().to(strava_create))),
                    )
                    .service(web::resource("/user").route(web::get().to(user)))
                    .service(
                        web::resource("/race_result_plot").route(web::get().to(race_result_plot)),
                    )
                    .service(
                        web::resource("/race_result_flag").route(web::get().to(race_result_flag)),
                    )
                    .service(
                        web::resource("/race_result_import")
                            .route(web::get().to(race_result_import)),
                    )
                    .service(
                        web::resource("/race_result_plot_demo")
                            .route(web::get().to(race_result_plot_demo)),
                    )
                    .service(
                        web::resource("/race_results_db")
                            .route(web::get().to(race_results_db))
                            .route(web::post().to(race_results_db_update)),
                    ),
            )
    })
    .bind(&format!("127.0.0.1:{}", CONFIG.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", CONFIG.port))
    .run()
    .await
    .map_err(Into::into)
}
