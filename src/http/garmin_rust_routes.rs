#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    http::StatusCode, AsyncResponder, FutureResponse, HttpMessage, HttpRequest, HttpResponse, Json,
    Query,
};
use failure::err_msg;
use futures::future::Future;
use rust_auth_server::auth_handler::LoggedUser;

use super::garmin_rust_app::AppState;
use crate::common::garmin_cli::{GarminCli, GarminCliObj};
use crate::common::garmin_config::GarminConfig;
use crate::common::garmin_correction_lap::GarminCorrectionListTrait;
use crate::common::garmin_file::GarminFile;
use crate::http::garmin_requests::{GarminCorrRequest, GarminHtmlRequest, GarminListRequest};
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::reports::garmin_file_report_txt;

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<String>,
    pub history: Option<String>,
}

fn proc_pattern_wrapper(request: FilterRequest) -> GarminHtmlRequest {
    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "latest".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let req = GarminCliObj::process_pattern(&filter_vec);

    GarminHtmlRequest {
        filter,
        history,
        ..req
    }
}

pub fn garmin(
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
pub struct GpsList {
    pub gps_list: Vec<String>,
}

#[derive(Serialize)]
pub struct TimeValue {
    pub time: String,
    pub value: f64,
}

pub fn garmin_list_gps_tracks(
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
pub struct HrData {
    pub hr_data: Vec<TimeValue>,
}

pub fn garmin_get_hr_data(
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
                    let gfile = match GarminFile::read_avro(&avro_file) {
                        Ok(g) => g,
                        Err(_) => {
                            let gps_file = format!("{}/{}", &config.gps_dir, file_name);
                            let corr_map = res1.map(|c| c.get_corr_list_map())?;
                            GarminParse::new().with_file(&gps_file, &corr_map)?
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
                    let gfile = match GarminFile::read_avro(&avro_file) {
                        Ok(g) => g,
                        Err(_) => {
                            let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                            let corr_map = res1.map(|c| c.get_corr_list_map())?;

                            GarminParse::new().with_file(&gps_file, &corr_map)?
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
