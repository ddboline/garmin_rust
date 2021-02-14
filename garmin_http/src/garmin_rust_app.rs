#![allow(clippy::needless_pass_by_value)]

use anyhow::Error;
use chrono::Utc;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::spawn, time::interval};
use warp::Filter;

use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

use crate::{
    errors::error_response,
    garmin_rust_routes::{
        add_garmin_correction, fitbit_activities, fitbit_activities_db,
        fitbit_activities_db_update, fitbit_activity_types, fitbit_auth, fitbit_bodyweight,
        fitbit_bodyweight_sync, fitbit_callback, fitbit_heartrate_api, fitbit_heartrate_cache,
        fitbit_heartrate_cache_update, fitbit_plots, fitbit_plots_demo, fitbit_profile,
        fitbit_refresh, fitbit_sync, fitbit_tcx_sync, garmin, garmin_connect_activities,
        garmin_connect_activities_db, garmin_connect_activities_db_update, garmin_connect_hr_api,
        garmin_connect_hr_sync, garmin_connect_sync, garmin_connect_user_summary, garmin_demo,
        garmin_sync, garmin_upload, heartrate_plots, heartrate_plots_demo,
        heartrate_statistics_plots, heartrate_statistics_plots_demo,
        heartrate_statistics_summary_db, heartrate_statistics_summary_db_update, race_result_flag,
        race_result_import, race_result_plot, race_result_plot_demo, race_results_db,
        race_results_db_update, scale_measurement, scale_measurement_update, strava_activities,
        strava_activities_db, strava_activities_db_update, strava_athlete, strava_auth,
        strava_callback, strava_create, strava_refresh, strava_sync, strava_update, strava_upload,
        user,
    },
    logged_user::{fill_from_db, get_secrets, TRIGGER_DB_UPDATE},
};

pub type ConnectProxy = Arc<Mutex<GarminConnectClient>>;

/// `AppState` is the application state shared between all the handlers
/// db can be used to send messages to the database workers, each running on
/// their own thread `user_list` contains a shared cache of previously
/// authorized users
#[derive(Clone)]
pub struct AppState {
    pub config: GarminConfig,
    pub db: PgPool,
    pub connect_proxy: ConnectProxy,
}

pub async fn close_connect_proxy(proxy: &ConnectProxy) -> Result<(), Error> {
    let mut proxy = proxy.lock().await;
    if proxy.last_used < Utc::now() - chrono::Duration::seconds(300) {
        proxy.close().await?;
    }
    Ok(())
}

/// Create the actix-web server.
/// Configuration is done through environment variables, see `GarminConfig` for
/// more information. `PgPool` is a wrapper around a connection pool.
/// We create several routes:
///    `/garmin` is the main route, providing the same functionality as the CLI
/// interface, while adding the ability of upload to strava, and
/// `/garmin/get_hr_pace` return structured json intended for separate analysis
pub async fn start_app() -> Result<(), Error> {
    async fn update_db(pool: PgPool, proxy: ConnectProxy) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            close_connect_proxy(&proxy).await.unwrap_or(());
            i.tick().await;
        }
    }

    let config = GarminConfig::get_config(None)?;

    TRIGGER_DB_UPDATE.set();
    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    let pool = PgPool::new(&config.pgurl);
    let proxy = Arc::new(Mutex::new(GarminConnectClient::new(config.clone())));

    spawn({
        let pool = pool.clone();
        let proxy = proxy.clone();
        async move { update_db(pool, proxy) }
    });

    run_app(&config, &pool, &proxy).await
}

async fn run_app(config: &GarminConfig, pool: &PgPool, proxy: &ConnectProxy) -> Result<(), Error> {
    let data = AppState {
        config: config.clone(),
        db: pool.clone(),
        connect_proxy: proxy.clone(),
    };

    let data = warp::any().map(move || data.clone());

    let index_path = warp::path("index.html")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(garmin)
        .boxed();
    let garmin_demo_path = warp::path("demo.html")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(garmin_demo)
        .boxed();
    let garmin_upload_path = warp::path("upload_file")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::query())
        .and(warp::multipart::form())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(garmin_upload)
        .boxed();
    let add_garmin_correction_path = warp::path("add_garmin_correction")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(add_garmin_correction)
        .boxed();
    let garmin_connect_sync_path = warp::path("garmin_connect_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_sync)
        .boxed();
    let garmin_connect_activities_path = warp::path("garmin_connect_activities")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_activities)
        .boxed();
    let garmin_connect_activities_db_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_activities_db);
    let garmin_connect_activities_db_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_activities_db_update);
    let garmin_connect_activities_db_path = warp::path("garmin_connect_activities_db")
        .and(garmin_connect_activities_db_get.or(garmin_connect_activities_db_post))
        .boxed();
    let garmin_connect_hr_sync_path = warp::path("garmin_connect_hr_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_hr_sync)
        .boxed();
    let garmin_connect_hr_api_path = warp::path("garmin_connect_hr_api")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_hr_api)
        .boxed();
    let garmin_connect_user_summary_path = warp::path("garmin_connect_user_summary")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_connect_user_summary)
        .boxed();
    let garmin_sync_path = warp::path("garmin_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(garmin_sync)
        .boxed();
    let strava_sync_path = warp::path("strava_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_sync)
        .boxed();
    let fitbit_auth_path = warp::path("auth")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_auth)
        .boxed();
    let fitbit_refresh_path = warp::path("refresh_auth")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_refresh)
        .boxed();
    let fitbit_callback_path = warp::path("callback")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and_then(fitbit_callback)
        .boxed();
    let fitbit_heartrate_api_path = warp::path("heartrate_api")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_heartrate_api)
        .boxed();
    let heartrate_cache_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_heartrate_cache);
    let heartrate_cache_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_heartrate_cache_update);
    let heartrate_cache_path = warp::path("heartrate_cache")
        .and(heartrate_cache_get.or(heartrate_cache_post))
        .boxed();
    let fitbit_sync_path = warp::path("sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_sync)
        .boxed();
    let fitbit_bodyweight_path = warp::path("bodyweight")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_bodyweight)
        .boxed();
    let fitbit_bodyweight_sync_path = warp::path("bodyweight_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_bodyweight_sync)
        .boxed();
    let fitbit_plots_path = warp::path("plots")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(fitbit_plots)
        .boxed();
    let fitbit_plots_demo_path = warp::path("plots_demo")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(fitbit_plots_demo)
        .boxed();
    let heartrate_statistics_plots_path = warp::path("heartrate_statistics_plots")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(heartrate_statistics_plots)
        .boxed();
    let heartrate_statistics_plots_demo_path = warp::path("heartrate_statistics_plots_demo")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(heartrate_statistics_plots_demo)
        .boxed();
    let heartrate_plots_path = warp::path("heartrate_plots")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(heartrate_plots)
        .boxed();
    let heartrate_plots_demo_path = warp::path("heartrate_plots_demo")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(heartrate_plots_demo)
        .boxed();
    let fitbit_tcx_sync_path = warp::path("fitbit_tcx_sync")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_tcx_sync)
        .boxed();
    let fitbit_activity_types_path = warp::path("fitbit_activity_types")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_activity_types)
        .boxed();
    let fitbit_activities_path = warp::path("fitbit_activities")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_activities)
        .boxed();
    let fitbit_activities_db_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_activities_db);
    let fitbit_activities_db_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_activities_db_update);
    let fitbit_activities_db_path = warp::path("fitbit_activities_db")
        .and(fitbit_activities_db_get.or(fitbit_activities_db_post))
        .boxed();
    let heartrate_statistics_summary_db_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(heartrate_statistics_summary_db);
    let heartrate_statistics_summary_db_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(heartrate_statistics_summary_db_update);
    let heartrate_statistics_summary_db_path = warp::path("heartrate_statistics_summary_db")
        .and(heartrate_statistics_summary_db_get.or(heartrate_statistics_summary_db_post))
        .boxed();
    let fitbit_profile_path = warp::path("profile")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(fitbit_profile)
        .boxed();
    let fitbit_path = warp::path("fitbit")
        .and(
            fitbit_auth_path
                .or(fitbit_refresh_path)
                .or(fitbit_callback_path)
                .or(fitbit_heartrate_api_path)
                .or(heartrate_cache_path)
                .or(fitbit_sync_path)
                .or(fitbit_bodyweight_path)
                .or(fitbit_bodyweight_sync_path)
                .or(fitbit_plots_path)
                .or(fitbit_plots_demo_path)
                .or(heartrate_statistics_plots_path)
                .or(heartrate_statistics_plots_demo_path)
                .or(heartrate_plots_path)
                .or(heartrate_plots_demo_path)
                .or(fitbit_tcx_sync_path)
                .or(fitbit_activity_types_path)
                .or(fitbit_activities_path)
                .or(fitbit_activities_db_path)
                .or(heartrate_statistics_summary_db_path)
                .or(fitbit_profile_path),
        )
        .boxed();
    let scale_measurements_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(scale_measurement);
    let scale_measurements_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(scale_measurement_update);
    let scale_measurements_path = warp::path("scale_measurements")
        .and(scale_measurements_get.or(scale_measurements_post))
        .boxed();
    let strava_auth_path = warp::path("auth")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_auth)
        .boxed();
    let strava_refresh_path = warp::path("refresh_auth")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_refresh)
        .boxed();
    let strava_callback_path = warp::path("callback")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_callback)
        .boxed();
    let strava_activities_path = warp::path("activities")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_activities)
        .boxed();
    let strava_athlete_path = warp::path("athlete")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_athlete)
        .boxed();
    let strava_activities_db_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_activities_db);
    let strava_activities_db_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_activities_db_update);
    let strava_activities_db_path = warp::path("activities_db")
        .and(strava_activities_db_get.or(strava_activities_db_post))
        .boxed();
    let strava_upload_path = warp::path("upload")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_upload)
        .boxed();
    let strava_update_path = warp::path("update")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_update)
        .boxed();
    let strava_create_path = warp::path("create")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(strava_create)
        .boxed();

    let strava_path = warp::path("strava")
        .and(
            strava_auth_path
                .or(strava_refresh_path)
                .or(strava_callback_path)
                .or(strava_activities_path)
                .or(strava_athlete_path)
                .or(strava_activities_db_path)
                .or(strava_upload_path)
                .or(strava_update_path)
                .or(strava_create_path),
        )
        .boxed();
    let user_path = warp::path("user")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and_then(user)
        .boxed();
    let race_result_plot_path = warp::path("race_result_plot")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(race_result_plot)
        .boxed();
    let race_result_flag_path = warp::path("race_result_flag")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(race_result_flag)
        .boxed();
    let race_result_import_path = warp::path("race_result_import")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(race_result_import)
        .boxed();
    let race_result_plot_demo_path = warp::path("race_result_plot_demo")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(data.clone())
        .and(warp::cookie::optional("session"))
        .and_then(race_result_plot_demo)
        .boxed();
    let race_results_db_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(race_results_db);
    let race_results_db_post = warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(race_results_db_update);
    let race_results_db_path = warp::path("race_results_db")
        .and(race_results_db_get.or(race_results_db_post))
        .boxed();

    let garmin_path = warp::path("garmin")
        .and(
            index_path
                .or(garmin_demo_path)
                .or(garmin_upload_path)
                .or(add_garmin_correction_path)
                .or(garmin_connect_sync_path)
                .or(garmin_connect_activities_path)
                .or(garmin_connect_activities_db_path)
                .or(garmin_connect_hr_sync_path)
                .or(garmin_connect_hr_api_path)
                .or(garmin_connect_user_summary_path)
                .or(garmin_sync_path)
                .or(strava_sync_path)
                .or(fitbit_path)
                .or(scale_measurements_path)
                .or(strava_path)
                .or(user_path)
                .or(race_result_plot_path)
                .or(race_result_flag_path)
                .or(race_result_import_path)
                .or(race_result_plot_demo_path)
                .or(race_results_db_path),
        )
        .boxed();

    let routes = garmin_path.recover(error_response);
    let addr: SocketAddr = format!("127.0.0.1:{}", config.port).parse()?;
    warp::serve(routes).bind(addr).await;
    Ok(())
}
