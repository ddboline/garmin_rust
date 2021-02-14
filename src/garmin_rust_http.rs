#![allow(clippy::needless_pass_by_value)]
use garmin_http::garmin_rust_app::start_app;

/// Start tokio and add our app to it
#[tokio::main]
async fn main() {
    env_logger::init();
    start_app().await.unwrap();
}
