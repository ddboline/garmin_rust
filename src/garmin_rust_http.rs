#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;
extern crate rust_auth_server;

use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Json, Query};
use chrono::Duration;
use failure::Error;
use rust_auth_server::auth_handler::LoggedUser;
use std::env;

use garmin_rust::common::garmin_cli::GarminCli;
use garmin_rust::common::garmin_config::GarminConfig;
use garmin_rust::common::garmin_correction_lap::GarminCorrectionList;
use garmin_rust::common::garmin_file;
use garmin_rust::parsers::garmin_parse;
use garmin_rust::reports::garmin_file_report_txt;
use garmin_rust::reports::garmin_report_options::GarminReportOptions;
use garmin_rust::utils::garmin_util::{get_list_of_files_from_db, get_pg_conn};

#[derive(Deserialize)]
struct FilterRequest {
    filter: Option<String>,
    history: Option<String>,
}

struct ProcPatternOutput(String, String, GarminReportOptions, Vec<String>);

fn proc_pattern_wrapper(request: FilterRequest) -> ProcPatternOutput {
    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (options, constraints) = GarminCli::process_pattern(&filter_vec);
    ProcPatternOutput(filter, history, options, constraints)
}

fn garmin(request: Query<FilterRequest>, user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let request = request.into_inner();

    let ProcPatternOutput(filter, history, options, constraints) = proc_pattern_wrapper(request);

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
struct TimeValue {
    time: String,
    value: f64,
}

fn garmin_list_gps_tracks(request: Query<FilterRequest>) -> Result<Json<GpsList>, Error> {
    let request = request.into_inner();

    let ProcPatternOutput(_, _, _, constraints) = proc_pattern_wrapper(request);

    let pgurl = GarminCli::new().with_config().config.pgurl;

    let gps_list = get_list_of_files_from_db(&get_pg_conn(&pgurl)?, &constraints)?;

    Ok(Json(GpsList { gps_list }))
}

#[derive(Serialize)]
struct HrData {
    hr_data: Vec<TimeValue>,
}

fn garmin_get_hr_data(request: Query<FilterRequest>) -> Result<Json<HrData>, Error> {
    let request = request.into_inner();

    let ProcPatternOutput(_, _, _, constraints) = proc_pattern_wrapper(request);

    let config = GarminCli::new().with_config().config;

    let pg_conn = get_pg_conn(&config.pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

    match file_list.len() {
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    garmin_parse::GarminParse::new()
                        .with_file(&gps_file, &corr_map)
                        .gfile
                }
            };

            Ok(Json(HrData {
                hr_data: gfile
                    .points
                    .iter()
                    .filter_map(|p| match p.heart_rate {
                        Some(hr) => Some(TimeValue {
                            time: p.time.clone(),
                            value: hr,
                        }),
                        None => None,
                    })
                    .collect(),
            }))
        }
        _ => Ok(Json(HrData {
            hr_data: Vec::new(),
        })),
    }
}

#[derive(Serialize)]
struct HrPace {
    hr: f64,
    pace: f64,
}

#[derive(Serialize)]
struct HrPaceList {
    hr_pace: Vec<HrPace>,
}

fn garmin_get_hr_pace(request: Query<FilterRequest>) -> Result<Json<HrPaceList>, Error> {
    let request = request.into_inner();

    let ProcPatternOutput(_, _, _, constraints) = proc_pattern_wrapper(request);

    let config = GarminCli::new().with_config().config;

    let pg_conn = get_pg_conn(&config.pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

    match file_list.len() {
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    garmin_parse::GarminParse::new()
                        .with_file(&gps_file, &corr_map)
                        .gfile
                }
            };

            let splits = garmin_file_report_txt::get_splits(&gfile, 400., "mi", true)?;

            Ok(Json(HrPaceList {
                hr_pace: splits
                    .iter()
                    .filter_map(|v| {
                        let s = v[1];
                        let h = v[2];
                        let pace = 4. * s / 60.;
                        if pace >= 5.5 && pace <= 20. {
                            Some(HrPace { hr: h, pace })
                        } else {
                            None
                        }
                    })
                    .collect(),
            }))
        }
        _ => Ok(Json(HrPaceList {
            hr_pace: Vec::new(),
        })),
    }
}

fn main() {
    let sys = actix::System::new("garmin");
    let config = GarminConfig::get_config();
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());

    server::new(move || {
        App::new()
            .middleware(IdentityService::new(
                CookieIdentityPolicy::new(secret.as_bytes())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age(Duration::days(1))
                    .secure(false), // this can only be true if you have https
            ))
            .resource("/garmin", |r| r.method(Method::GET).with(garmin))
            .resource("/garmin/list_gps_tracks", |r| {
                r.method(Method::GET).with(garmin_list_gps_tracks)
            })
            .resource("/garmin/get_hr_data", |r| {
                r.method(Method::GET).with(garmin_get_hr_data)
            })
            .resource("/get_hr_pace", |r| {
                r.method(Method::GET).with(garmin_get_hr_pace)
            })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", config.port))
    .start();

    let _ = sys.run();
}
