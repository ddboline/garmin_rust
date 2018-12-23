#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;

use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Json, Query};

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

#[derive(Serialize)]
struct GpsList {
    gps_list: Vec<String>,
}

#[derive(Serialize)]
struct HrData {
    hr_data: Vec<(String, f64)>,
}

#[derive(Serialize)]
struct HrPace {
    hr_pace: Vec<(f64, f64)>,
}

fn garmin_list_gps_tracks(request: Query<FilterRequest>) -> Result<Json<GpsList>, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (options, constraints) = GarminCli::process_pattern(&filter_vec);

    let config = GarminCli::new().with_config();

    let pg_conn = get_pg_conn(&config.pgurl)?;

    Ok(get_list_of_files_from_db(&pg_conn, &constraints)?)
}

fn garmin_get_hr_data(request: Query<FilterRequest>) -> Result<Json<HrData>, Error> {}

fn garmin_get_hr_pace(request: Query<FilterRequest>) -> Result<Json<HrPace>, Error> {}

fn main() {
    let sys = actix::System::new("garmin");
    let config = GarminConfig::get_config();

    server::new(|| {
        App::new()
            .resource("/garmin", |r| r.method(Method::GET).with(garmin))
            .resource("/garmin/list_gps_tracks", |r| {
                r.method(Method::GET).with(garmin_list_gps_tracks)
            })
            .resource("/garmin/get_hr_data", |r| {
                r.method(Method::GET).with(garmin_get_hr_data)
            })
            .resource("/garmin/get_hr_pace", |r| {
                r.method(Method::GET).with(garmin_get_hr_pace)
            })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap()
    .start();

    let _ = sys.run();
}
