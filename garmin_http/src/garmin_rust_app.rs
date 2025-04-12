#![allow(clippy::needless_pass_by_value)]

use axum::http::{header::CONTENT_TYPE, Method, StatusCode};
use log::{debug, error, info};
use maplit::hashset;
use notify::{
    recommended_watcher, Event, EventHandler, EventKind, INotifyWatcher, RecursiveMode,
    Result as NotifyResult, Watcher,
};
use reqwest::{Client, ClientBuilder};
use stack_string::format_sstr;
use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    net::TcpListener,
    sync::watch::{channel, Receiver, Sender},
    task::spawn,
    time::{interval, sleep, Duration},
};
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use garmin_cli::{
    garmin_cli::GarminCli,
    garmin_cli_opts::{GarminCliOpts, GarminConnectSyncOutput},
};
use garmin_lib::{errors::GarminError, garmin_config::GarminConfig};
use garmin_models::garmin_correction_lap::GarminCorrectionMap;
use garmin_utils::pgpool::PgPool;

use crate::{
    errors::ServiceError as Error,
    garmin_rust_routes::{get_garmin_path, ApiDoc},
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

    fn set_watcher(mut self, directory: &Path) -> Result<Self, GarminError> {
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
        if let Ok(GarminConnectSyncOutput {
            filenames,
            input_files,
            dates,
        }) = GarminCliOpts::sync_with_garmin_connect(cli, &None, None, None, false).await
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
    let port = config.port;

    run_app(&config, &pool, port).await
}

async fn run_app(config: &GarminConfig, pool: &PgPool, port: u32) -> Result<(), Error> {
    let app = AppState {
        config: config.clone(),
        db: pool.clone(),
        client: Arc::new(
            ClientBuilder::new()
                .build()
                .map_err(Into::<GarminError>::into)?,
        ),
    };

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE])
        .allow_origin(Any);

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(get_garmin_path(&app))
        .split_for_parts();

    let spec_json = serde_json::to_string_pretty(&api).map_err(Into::<GarminError>::into)?;
    let spec_yaml = serde_yml::to_string(&api).map_err(Into::<GarminError>::into)?;

    let router = router
        .route(
            "/garmin/openapi/json",
            axum::routing::get(|| async move {
                (
                    StatusCode::OK,
                    [(CONTENT_TYPE, mime::APPLICATION_JSON.essence_str())],
                    spec_json,
                )
            }),
        )
        .route(
            "/garmin/openapi/yaml",
            axum::routing::get(|| async move {
                (StatusCode::OK, [(CONTENT_TYPE, "text/yaml")], spec_yaml)
            }),
        )
        .layer(cors);

    let host = &config.host;

    let addr: SocketAddr = format_sstr!("{host}:{port}").parse()?;
    debug!("{addr:?}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, router.into_make_service())
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use stack_string::format_sstr;
    use std::env::{remove_var, set_var};

    use garmin_lib::garmin_config::GarminConfig;
    use garmin_utils::pgpool::PgPool;

    use crate::{
        errors::ServiceError as Error,
        garmin_rust_app::run_app,
        logged_user::{JWT_SECRET, KEY_LENGTH, SECRET_KEY, get_random_key},
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn test_run_app() -> Result<(), Error> {
        unsafe {
            set_var("TESTENV", "true");
        }

        let mut secret_key = [0u8; KEY_LENGTH];
        secret_key.copy_from_slice(&get_random_key());

        JWT_SECRET.set(secret_key);
        SECRET_KEY.set(secret_key);

        let test_port: u32 = 12345;
        unsafe {
            set_var("PORT", test_port.to_string());
        }
        let config = GarminConfig::get_config(None)?;

        let pool = PgPool::new(&config.pgurl)?;

        tokio::task::spawn(async move {
            env_logger::init();
            run_app(&config, &pool, test_port).await.unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let client = reqwest::Client::builder().cookie_store(true).build()?;

        let url = format_sstr!("http://localhost:{test_port}/garmin/openapi/yaml");
        let spec_yaml = client
            .get(url.as_str())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        std::fs::write("../scripts/openapi.yaml", &spec_yaml)?;

        unsafe {
            remove_var("TESTENV");
        }
        Ok(())
    }
}
