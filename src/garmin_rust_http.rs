#![allow(clippy::needless_pass_by_value)]
use garmin_http::garmin_rust_app::start_app;

/// Start actix system and add our app to it
fn main() {
    let sys = actix_rt::System::new("garmin");

    start_app();

    let _ = sys.run();
}
