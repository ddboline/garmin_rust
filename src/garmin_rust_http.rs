#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;

use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Query};

use failure::Error;

use garmin_rust::garmin_cli::GarminCli;
use garmin_rust::garmin_config::GarminConfig;

#[derive(Debug, Deserialize)]
struct FilterRequest {
    filter: Option<String>,
    history: Option<String>,
}

fn garmin(request: Query<FilterRequest>) -> Result<HttpResponse, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (options, constraints) = GarminCli::process_pattern(&filter_vec);

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(GarminCli::new().with_config().run_html(
            &options,
            &constraints,
            &filter,
            &history,
        )?);
    Ok(resp)
}

fn main() {
    let sys = actix::System::new("garmin");
    let config = GarminConfig::get_config();

    server::new(|| App::new().resource("/garmin", |r| r.method(Method::GET).with(garmin)))
        .bind(&format!("127.0.0.1:{}", config.port))
        .unwrap()
        .start();

    let _ = sys.run();
}
