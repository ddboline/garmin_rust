#![allow(clippy::needless_pass_by_value)]

#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;
extern crate rust_auth_server;

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{
    http::Method, http::StatusCode, server, App, AsyncResponder, FutureResponse, HttpMessage,
    HttpRequest, HttpResponse, Json, Query,
};
use chrono::Duration;
use failure::err_msg;
use futures::future::Future;
use rust_auth_server::auth_handler::LoggedUser;
use std::env;

use garmin_rust::common::garmin_cli::{
    GarminCli, GarminCorrRequest, GarminHtmlRequest, GarminListRequest,
};
use garmin_rust::common::garmin_config::GarminConfig;
use garmin_rust::common::garmin_file;
use garmin_rust::common::pgpool::PgPool;
use garmin_rust::parsers::garmin_parse;
use garmin_rust::reports::garmin_file_report_txt;

pub struct AppState {
    pub db: Addr<PgPool>,
}

#[derive(Deserialize)]
struct FilterRequest {
    filter: Option<String>,
    history: Option<String>,
}

fn proc_pattern_wrapper(request: FilterRequest) -> GarminHtmlRequest {
    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let req = GarminCli::process_pattern(&filter_vec);

    GarminHtmlRequest {
        filter,
        history,
        ..req
    }
}

fn garmin(
    query: Query<FilterRequest>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        request
            .body()
            .from_err()
            .and_then(move |_| Ok(HttpResponse::Unauthorized().json("Unauthorized")))
            .responder()
    } else {
        let query = query.into_inner();

        let req = proc_pattern_wrapper(query);

        request
            .state()
            .db
            .send(req)
            .from_err()
            .and_then(move |res| match res {
                Ok(body) => {
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
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

fn garmin_list_gps_tracks(
    query: Query<FilterRequest>,
    request: HttpRequest<AppState>,
) -> FutureResponse<Json<GpsList>> {
    let query = query.into_inner();

    let req: GarminListRequest = proc_pattern_wrapper(query).into();

    request
        .state()
        .db
        .send(req)
        .from_err()
        .and_then(move |res| match res {
            Ok(gps_list) => Ok(Json(GpsList { gps_list })),
            Err(err) => Err(err.into()),
        })
        .responder()
}

#[derive(Serialize)]
struct HrData {
    hr_data: Vec<TimeValue>,
}

fn garmin_get_hr_data(
    query: Query<FilterRequest>,
    request: HttpRequest<AppState>,
) -> FutureResponse<Json<HrData>> {
    let query = query.into_inner();

    let req: GarminListRequest = proc_pattern_wrapper(query).into();

    request
        .state()
        .db
        .send(req)
        .from_err()
        .join(request.state().db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| match res0 {
            Ok(file_list) => match file_list.len() {
                1 => {
                    let config = GarminConfig::get_config(None);
                    let file_name = file_list
                        .get(0)
                        .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                    let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                    let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                        Ok(g) => g,
                        Err(_) => {
                            let gps_file = format!("{}/{}", &config.gps_dir, file_name);
                            let corr_map = res1.map(|c| c.get_corr_list_map())?;
                            garmin_parse::GarminParse::new().with_file(&gps_file, &corr_map)?
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
            },
            Err(err) => Err(err.into()),
        })
        .responder()
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

fn garmin_get_hr_pace(
    query: Query<FilterRequest>,
    request: HttpRequest<AppState>,
) -> FutureResponse<Json<HrPaceList>> {
    let query = query.into_inner();

    let req: GarminListRequest = proc_pattern_wrapper(query).into();

    request
        .state()
        .db
        .send(req)
        .from_err()
        .join(request.state().db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| match res0 {
            Ok(file_list) => match file_list.len() {
                1 => {
                    let config = GarminConfig::get_config(None);
                    let file_name = file_list
                        .get(0)
                        .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                    let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                    let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                        Ok(g) => g,
                        Err(_) => {
                            let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                            let corr_map = res1.map(|c| c.get_corr_list_map())?;

                            garmin_parse::GarminParse::new().with_file(&gps_file, &corr_map)?
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
            },
            Err(err) => Err(err.into()),
        })
        .responder()
}

fn main() {
    let sys = actix::System::new("garmin");
    let config = GarminConfig::get_config(None);
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());
    let nconn = config.n_db_workers;
    let pool = PgPool::new(&config.pgurl);

    let addr: Addr<PgPool> = SyncArbiter::start(nconn, move || pool.clone());

    server::new(move || {
        App::with_state(AppState { db: addr.clone() })
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
            .resource("/garmin/get_hr_pace", |r| {
                r.method(Method::GET).with(garmin_get_hr_pace)
            })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", config.port))
    .start();

    let _ = sys.run();
}
