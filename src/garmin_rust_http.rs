#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;

use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Query};

use failure::Error;

use garmin_rust::garmin_cli;

#[derive(Debug, Deserialize)]
struct FilterRequest {
    filter: Option<String>,
    history: Option<String>,
}

fn garmin(request: Query<FilterRequest>) -> Result<HttpResponse, Error> {
    let filter = request.filter.clone().unwrap_or("sport".to_string());
    let history = request.history.clone().unwrap_or("sport".to_string());

    let filter_vec: Vec<String> = filter.split(",").map(|x| x.to_string()).collect();

    let (options, constraints) = garmin_cli::process_pattern(&filter_vec);

    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(garmin_cli::run_html(
            &options,
            &constraints,
            &filter,
            &history,
        )?))
}

fn main() {
    let sys = actix::System::new("garmin");
    let config = garmin_cli::get_garmin_config();

    server::new(|| App::new().resource("/garmin", |r| r.method(Method::GET).with(garmin)))
        .bind(&format!("127.0.0.1:{}", config.port))
        .unwrap()
        .start();

    let _ = sys.run();
}
