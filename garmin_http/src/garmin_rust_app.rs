#![allow(clippy::needless_pass_by_value)]

use anyhow::Error;
use log::{error, info};
use maplit::hashset;
use notify::{
    recommended_watcher, Event, EventHandler, EventKind, INotifyWatcher, RecursiveMode,
    Result as NotifyResult, Watcher,
};
use reqwest::{Client, ClientBuilder};
use rweb::{
    filters::BoxedFilter,
    http::header::CONTENT_TYPE,
    openapi::{self, Info},
    Filter, Reply,
};
use stack_string::format_sstr;
use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    sync::watch::{channel, Receiver, Sender},
    task::spawn,
    time::{interval, sleep, Duration},
};

use garmin_cli::{garmin_cli::GarminCli, garmin_cli_opts::GarminCliOpts};
use garmin_lib::garmin_config::GarminConfig;
use garmin_models::garmin_correction_lap::GarminCorrectionMap;
use garmin_utils::pgpool::PgPool;

use crate::{
    errors::error_response,
    garmin_rust_routes::{
        add_garmin_correction, fitbit_activities_db, fitbit_activities_db_update,
        fitbit_heartrate_cache, fitbit_heartrate_cache_update, fitbit_plots, fitbit_plots_demo,
        garmin, garmin_connect_activities_db, garmin_connect_activities_db_update, garmin_demo,
        garmin_scripts_demo_js, garmin_scripts_js, garmin_sync, garmin_upload, heartrate_plots,
        heartrate_plots_demo, heartrate_statistics_plots, heartrate_statistics_plots_demo,
        heartrate_statistics_summary_db, heartrate_statistics_summary_db_update, initialize_map_js,
        line_plot_js, race_result_flag, race_result_import, race_result_plot,
        race_result_plot_demo, race_results_db, race_results_db_update, scale_measurement,
        scale_measurement_manual, scale_measurement_manual_input, scale_measurement_update,
        scatter_plot_js, scatter_plot_with_lines_js, strava_activities, strava_activities_db,
        strava_activities_db_update, strava_athlete, strava_auth, strava_callback, strava_create,
        strava_refresh, strava_sync, strava_update, strava_upload, time_series_js, user,
    },
    logged_user::{fill_from_db, get_secrets},
};

/// `AppState` is the application state shared between all the handlers
/// db can be used to send messages to the database workers, each running on
/// their own thread `user_list` contains a shared cache of previously
/// authorized users
#[derive(Clone)]
pub struct AppState {
    pub config: GarminConfig,
    pub db: PgPool,
    pub client: Arc<Client>,
}

#[derive(Clone)]
struct Notifier {
    paths_to_check: HashSet<PathBuf>,
    send: Sender<HashSet<PathBuf>>,
    recv: Receiver<HashSet<PathBuf>>,
    watcher: Option<Arc<INotifyWatcher>>,
}

impl Notifier {
    fn new(config: &GarminConfig) -> Self {
        let har_file = config.download_directory.join("connect.garmin.com.har");
        let strava_har_file = config.download_directory.join("www.strava.com.har");
        let data_directory = &config.garmin_connect_import_directory;
        let activites_json = data_directory.join("activities.json");
        let heartrate_json = data_directory.join("heartrates.json");
        let paths_to_check = hashset! {har_file, activites_json, heartrate_json, strava_har_file};
        let (send, recv) = channel::<HashSet<PathBuf>>(HashSet::new());
        Self {
            paths_to_check,
            send,
            recv,
            watcher: None,
        }
    }

    fn set_watcher(mut self, directory: &Path) -> Result<Self, Error> {
        let watcher = recommended_watcher(self.clone())
            .and_then(|mut w| w.watch(directory, RecursiveMode::Recursive).map(|()| w))?;
        self.watcher = Some(Arc::new(watcher));
        Ok(self)
    }
}

impl EventHandler for Notifier {
    fn handle_event(&mut self, event: NotifyResult<Event>) {
        match event {
            Ok(event) => match event.kind {
                EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) => {
                    if event.paths.iter().any(|p| self.paths_to_check.contains(p)) {
                        info!("got event kind {:?} paths {:?}", event.kind, event.paths);
                        let new_paths: HashSet<_> = event
                            .paths
                            .into_iter()
                            .filter(|p| self.paths_to_check.contains(p))
                            .collect();
                        self.send.send_replace(new_paths);
                    }
                }
                _ => (),
            },
            Err(e) => error!("Error {e}"),
        }
    }
}

/// Create the server.
/// Configuration is done through environment variables, see `GarminConfig` for
/// more information. `PgPool` is a wrapper around a connection pool.
/// We create several routes:
///    `/garmin` is the main route, providing the same functionality as the CLI
/// interface, while adding the ability of upload to strava, and
/// `/garmin/get_hr_pace` return structured json intended for separate analysis
/// # Errors
/// Returns error if server init fails
pub async fn start_app() -> Result<(), Error> {
    async fn update_db(pool: PgPool) {
        let mut i = interval(std::time::Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            i.tick().await;
        }
    }
    async fn run_connect_sync(cli: &GarminCli) {
        if let Ok((filenames, input_files, dates)) =
            GarminCliOpts::sync_with_garmin_connect(cli, &None, None, None, false).await
        {
            if !filenames.is_empty() || !input_files.is_empty() || !dates.is_empty() {
                info!("processed filenames {filenames:?} from {input_files:?} and dates {dates:?}");
                for line in cli.sync_everything().await.unwrap_or(Vec::new()) {
                    info!("{line}");
                }
            }
        }
    }
    async fn check_downloads(cli: GarminCli, mut notifier: Notifier) {
        run_connect_sync(&cli).await;
        while notifier.recv.changed().await.is_ok() {
            sleep(Duration::from_secs(10)).await;
            run_connect_sync(&cli).await;
            notifier.recv.mark_changed();
        }
    }

    let config = GarminConfig::get_config(None)?;

    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    let pool = PgPool::new(&config.pgurl)?;

    let notifier = Notifier::new(&config).set_watcher(&config.download_directory)?;

    spawn({
        let pool = pool.clone();
        async move {
            update_db(pool).await;
        }
    });
    spawn({
        let pool = pool.clone();
        let corr = GarminCorrectionMap::new();
        let cli = GarminCli {
            opts: Some(garmin_cli::garmin_cli::GarminCliOptions::Connect {
                data_directory: None,
                start_date: None,
                end_date: None,
            }),
            pool: pool.clone(),
            config: config.clone(),
            corr,
            ..GarminCli::default()
        };
        async move {
            check_downloads(cli, notifier).await;
        }
    });

    run_app(&config, &pool).await
}

fn get_garmin_path(app: &AppState) -> BoxedFilter<(impl Reply,)> {
    let index_path = garmin(app.clone()).boxed();
    let garmin_demo_path = garmin_demo(app.clone()).boxed();
    let garmin_upload_path = garmin_upload(app.clone()).boxed();
    let add_garmin_correction_path = add_garmin_correction(app.clone()).boxed();
    let garmin_connect_activities_db_get = garmin_connect_activities_db(app.clone()).boxed();
    let garmin_connect_activities_db_post =
        garmin_connect_activities_db_update(app.clone()).boxed();
    let garmin_connect_activities_db_path = garmin_connect_activities_db_get
        .or(garmin_connect_activities_db_post)
        .boxed();
    let garmin_sync_path = garmin_sync(app.clone()).boxed();
    let strava_sync_path = strava_sync(app.clone()).boxed();
    let heartrate_cache_get = fitbit_heartrate_cache(app.clone()).boxed();
    let heartrate_cache_post = fitbit_heartrate_cache_update(app.clone()).boxed();
    let heartrate_cache_path = heartrate_cache_get.or(heartrate_cache_post).boxed();
    let fitbit_plots_path = fitbit_plots(app.clone()).boxed();
    let fitbit_plots_demo_path = fitbit_plots_demo(app.clone()).boxed();
    let heartrate_statistics_plots_path = heartrate_statistics_plots(app.clone()).boxed();
    let heartrate_statistics_plots_demo_path = heartrate_statistics_plots_demo(app.clone()).boxed();
    let heartrate_plots_path = heartrate_plots(app.clone()).boxed();
    let heartrate_plots_demo_path = heartrate_plots_demo(app.clone()).boxed();
    let fitbit_activities_db_get = fitbit_activities_db(app.clone()).boxed();
    let fitbit_activities_db_post = fitbit_activities_db_update(app.clone()).boxed();
    let fitbit_activities_db_path = fitbit_activities_db_get
        .or(fitbit_activities_db_post)
        .boxed();
    let heartrate_statistics_summary_db_get = heartrate_statistics_summary_db(app.clone()).boxed();
    let heartrate_statistics_summary_db_post =
        heartrate_statistics_summary_db_update(app.clone()).boxed();
    let heartrate_statistics_summary_db_path = heartrate_statistics_summary_db_get
        .or(heartrate_statistics_summary_db_post)
        .boxed();
    let fitbit_path = heartrate_cache_path
        .or(fitbit_plots_path)
        .or(fitbit_plots_demo_path)
        .or(heartrate_statistics_plots_path)
        .or(heartrate_statistics_plots_demo_path)
        .or(heartrate_plots_path)
        .or(heartrate_plots_demo_path)
        .or(fitbit_activities_db_path)
        .or(heartrate_statistics_summary_db_path)
        .boxed();
    let scale_measurements_get = scale_measurement(app.clone()).boxed();
    let scale_measurements_post = scale_measurement_update(app.clone()).boxed();
    let scale_measurement_manual_path = scale_measurement_manual(app.clone()).boxed();
    let scale_measurement_manual_input_path = scale_measurement_manual_input().boxed();
    let scale_measurements_path = scale_measurements_get.or(scale_measurements_post).boxed();
    let strava_auth_path = strava_auth(app.clone()).boxed();
    let strava_refresh_path = strava_refresh(app.clone()).boxed();
    let strava_callback_path = strava_callback(app.clone()).boxed();
    let strava_activities_path = strava_activities(app.clone()).boxed();
    let strava_athlete_path = strava_athlete(app.clone()).boxed();
    let strava_activities_db_get = strava_activities_db(app.clone()).boxed();
    let strava_activities_db_post = strava_activities_db_update(app.clone()).boxed();
    let strava_activities_db_path = strava_activities_db_get
        .or(strava_activities_db_post)
        .boxed();
    let strava_upload_path = strava_upload(app.clone()).boxed();
    let strava_update_path = strava_update(app.clone()).boxed();
    let strava_create_path = strava_create(app.clone()).boxed();

    let strava_path = strava_auth_path
        .or(strava_refresh_path)
        .or(strava_callback_path)
        .or(strava_activities_path)
        .or(strava_athlete_path)
        .or(strava_activities_db_path)
        .or(strava_upload_path)
        .or(strava_update_path)
        .or(strava_create_path)
        .boxed();

    let user_path = user().boxed();
    let race_result_plot_path = race_result_plot(app.clone()).boxed();
    let race_result_flag_path = race_result_flag(app.clone()).boxed();
    let race_result_import_path = race_result_import(app.clone()).boxed();
    let race_result_plot_demo_path = race_result_plot_demo(app.clone()).boxed();
    let race_results_db_get = race_results_db(app.clone()).boxed();
    let race_results_db_post = race_results_db_update(app.clone()).boxed();
    let race_results_db_path = race_results_db_get.or(race_results_db_post).boxed();

    let garmin_scripts_js_path = garmin_scripts_js().boxed();
    let garmin_scripts_demo_js_path = garmin_scripts_demo_js().boxed();
    let line_plot_js_path = line_plot_js().boxed();
    let scatter_plot_js_path = scatter_plot_js().boxed();
    let scatter_plot_with_lines_js_path = scatter_plot_with_lines_js().boxed();
    let time_series_js_path = time_series_js().boxed();
    let initialize_map_js_path = initialize_map_js().boxed();

    index_path
        .or(garmin_demo_path)
        .or(garmin_upload_path)
        .or(add_garmin_correction_path)
        .or(garmin_connect_activities_db_path)
        .or(garmin_sync_path)
        .or(strava_sync_path)
        .or(fitbit_path)
        .or(scale_measurement_manual_path)
        .or(scale_measurement_manual_input_path)
        .or(scale_measurements_path)
        .or(strava_path)
        .or(user_path)
        .or(race_result_plot_path)
        .or(race_result_flag_path)
        .or(race_result_import_path)
        .or(race_result_plot_demo_path)
        .or(race_results_db_path)
        .or(garmin_scripts_js_path)
        .or(garmin_scripts_demo_js_path)
        .or(line_plot_js_path)
        .or(scatter_plot_js_path)
        .or(scatter_plot_with_lines_js_path)
        .or(time_series_js_path)
        .or(initialize_map_js_path)
        .boxed()
}

async fn run_app(config: &GarminConfig, pool: &PgPool) -> Result<(), Error> {
    let app = AppState {
        config: config.clone(),
        db: pool.clone(),
        client: Arc::new(ClientBuilder::new().build()?),
    };

    let (spec, garmin_path) = openapi::spec()
        .info(Info {
            title: "Fitness Activity WebApp".into(),
            description: "Web Frontend for Fitness Activities".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..Info::default()
        })
        .build(|| get_garmin_path(&app));
    let spec = Arc::new(spec);
    let spec_json_path = rweb::path!("garmin" / "openapi" / "json")
        .and(rweb::path::end())
        .map({
            let spec = spec.clone();
            move || rweb::reply::json(spec.as_ref())
        });

    let spec_yaml = serde_yml::to_string(spec.as_ref())?;
    let spec_yaml_path = rweb::path!("garmin" / "openapi" / "yaml")
        .and(rweb::path::end())
        .map(move || {
            let reply = rweb::reply::html(spec_yaml.clone());
            rweb::reply::with_header(reply, CONTENT_TYPE, "text/yaml")
        });

    let routes = garmin_path
        .or(spec_json_path)
        .or(spec_yaml_path)
        .recover(error_response);
    let addr: SocketAddr = format_sstr!("{}:{}", config.host, config.port).parse()?;
    rweb::serve(routes).bind(addr).await;
    Ok(())
}
