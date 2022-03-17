#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

use garmin_http::garmin_rust_app::start_app;

/// Start tokio and add our app to it
#[tokio::main]
async fn main() {
    env_logger::init();
    start_app().await.unwrap();
}
