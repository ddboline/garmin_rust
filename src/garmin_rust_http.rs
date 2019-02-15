#![allow(clippy::needless_pass_by_value)]

use garmin_rust::http::garmin_rust_app::start_app;

fn main() {
    let sys = actix::System::new("garmin");

    start_app();

    let _ = sys.run();
}
