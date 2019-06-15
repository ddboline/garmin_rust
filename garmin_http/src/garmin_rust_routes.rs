#![allow(clippy::needless_pass_by_value)]

use actix_web::http::StatusCode;
use actix_web::web::{Data, Query};
use actix_web::HttpResponse;
use chrono::{Date, Datelike, Local};
use failure::{err_msg, Error};
use futures::future::Future;
use serde::Serialize;
use std::string::ToString;

use garmin_lib::common::garmin_cli::{GarminCli, GarminCliObj, GarminRequest};
use garmin_lib::common::garmin_correction_lap::GarminCorrectionListTrait;
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use garmin_lib::reports::garmin_file_report_txt::get_splits;

use super::logged_user::LoggedUser;

use super::garmin_rust_app::AppState;
use crate::garmin_requests::{GarminCorrRequest, GarminHtmlRequest, GarminListRequest};
use crate::CONFIG;

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<String>,
    pub history: Option<String>,
}

fn proc_pattern_wrapper(request: FilterRequest) -> GarminHtmlRequest {
    let local: Date<Local> = Local::today();
    let year = local.year();
    let month = local.month();
    let (prev_year, prev_month) = if month > 1 {
        (year, month - 1)
    } else {
        (year - 1, 12)
    };
    let default_string = format!(
        "{:04}-{:02},{:04}-{:02},week",
        prev_year, prev_month, year, month
    );

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request
        .history
        .unwrap_or_else(|| format!("{};latest;sport", default_string));

    let filter_vec: Vec<String> = filter.split(',').map(ToString::to_string).collect();

    let req = GarminCliObj::process_pattern(&filter_vec);

    GarminHtmlRequest(GarminRequest {
        filter,
        history,
        ..req
    })
}

fn form_http_response(body: String) -> HttpResponse {
    HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body)
}

pub fn garmin(
    query: Query<FilterRequest>,
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    let grec = proc_pattern_wrapper(query);

    state.db.send(grec).from_err().and_then(move |res| {
        res.and_then(|body| {
            if !state.user_list.is_authorized(&user) {
                return Ok(HttpResponse::Unauthorized()
                    .json(format!("Unauthorized {:?}", state.user_list)));
            }
            Ok(form_http_response(body))
        })
    })
}

#[derive(Serialize)]
pub struct GpsList {
    pub gps_list: Vec<String>,
}

#[derive(Serialize)]
pub struct TimeValue {
    pub time: String,
    pub value: f64,
}

fn to_json<T>(js: &T) -> Result<HttpResponse, Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json2(js))
}

pub fn garmin_list_gps_tracks(
    query: Query<FilterRequest>,
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query).into();

    state.db.send(greq).from_err().and_then(move |res| {
        res.and_then(|gps_list| {
            if !state.user_list.is_authorized(&user) {
                return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
            }
            let glist = GpsList { gps_list };
            to_json(&glist)
        })
    })
}

#[derive(Serialize)]
pub struct HrData {
    pub hr_data: Vec<TimeValue>,
}

pub fn garmin_get_hr_data(
    query: Query<FilterRequest>,
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query).into();

    state
        .db
        .send(greq)
        .from_err()
        .join(state.db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| {
            res0.and_then(|file_list| {
                if !state.user_list.is_authorized(&user) {
                    return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
                }
                let hr_data = match file_list.len() {
                    1 => {
                        let config = &CONFIG;
                        let file_name = file_list
                            .get(0)
                            .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                        let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                        let gfile = match GarminFile::read_avro(&avro_file) {
                            Ok(g) => g,
                            Err(_) => {
                                let gps_file = format!("{}/{}", &config.gps_dir, file_name);
                                let corr_map = res1.map(|c| c.get_corr_list_map())?;
                                GarminParse::new().with_file(&gps_file, &corr_map)?
                            }
                        };

                        gfile
                            .points
                            .iter()
                            .filter_map(|point| match point.heart_rate {
                                Some(heart_rate) => Some(TimeValue {
                                    time: point.time.clone(),
                                    value: heart_rate,
                                }),
                                None => None,
                            })
                            .collect()
                    }
                    _ => Vec::new(),
                };
                let hdata = HrData { hr_data };
                to_json(&hdata)
            })
        })
}

#[derive(Serialize)]
pub struct HrPace {
    pub hr: f64,
    pub pace: f64,
}

#[derive(Serialize)]
pub struct HrPaceList {
    pub hr_pace: Vec<HrPace>,
}

pub fn garmin_get_hr_pace(
    query: Query<FilterRequest>,
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query).into();

    state
        .db
        .send(greq)
        .from_err()
        .join(state.db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| {
            res0.and_then(|file_list| {
                if !state.user_list.is_authorized(&user) {
                    return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
                }

                let hrpace = match file_list.len() {
                    1 => {
                        let config = &CONFIG;
                        let file_name = &file_list[0];
                        let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                        let gfile = match GarminFile::read_avro(&avro_file) {
                            Ok(g) => g,
                            Err(_) => {
                                let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                                let corr_map = res1.map(|c| c.get_corr_list_map())?;

                                GarminParse::new().with_file(&gps_file, &corr_map)?
                            }
                        };

                        let splits = get_splits(&gfile, 400., "mi", true)?;

                        HrPaceList {
                            hr_pace: splits
                                .iter()
                                .filter_map(|v| {
                                    let s = v.time_value;
                                    let h = v.avg_heart_rate.unwrap_or(0.0);
                                    let pace = 4. * s / 60.;
                                    if pace >= 5.5 && pace <= 20. {
                                        Some(HrPace { hr: h, pace })
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        }
                    }
                    _ => HrPaceList {
                        hr_pace: Vec::new(),
                    },
                };
                to_json(&hrpace)
            })
        })
}
