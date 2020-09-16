#![allow(clippy::needless_pass_by_value)]
use garmin_http::garmin_rust_app::start_app;

/// Start actix system and add our app to it
#[actix_rt::main]
async fn main() {
    env_logger::init();
    start_app().await.unwrap();
}
