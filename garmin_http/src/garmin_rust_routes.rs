#![allow(clippy::needless_pass_by_value)]

use actix_multipart::{Field, Multipart, MultipartError};
use actix_web::http::StatusCode;
use actix_web::web::{block, Data, Json, Query};
use actix_web::HttpResponse;
use failure::{err_msg, format_err, Error};
use futures::future::{err, Either, Future};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::string::ToString;
use tempdir::TempDir;

use garmin_lib::common::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use garmin_lib::reports::garmin_file_report_html::generate_history_buttons;
use garmin_lib::reports::garmin_file_report_txt::get_splits;
use garmin_lib::utils::iso_8601_datetime::convert_datetime_to_str;

use super::logged_user::LoggedUser;

use super::garmin_rust_app::AppState;
use crate::garmin_requests::{
    FitbitAuthRequest, FitbitBodyWeightFatRequest, FitbitBodyWeightFatUpdateRequest,
    FitbitCallbackRequest, FitbitHeartrateApiRequest, FitbitHeartrateDbRequest,
    FitbitHeartratePlotRequest, FitbitSyncRequest, GarminConnectSyncRequest, GarminCorrRequest,
    GarminHtmlRequest, GarminListRequest, GarminSyncRequest, GarminUploadRequest,
    ScaleMeasurementPlotRequest, ScaleMeasurementRequest, ScaleMeasurementUpdateRequest,
    StravaActivitiesRequest, StravaAuthRequest, StravaCallbackRequest, StravaSyncRequest,
    StravaUpdateRequest, StravaUploadRequest,
};
use crate::CONFIG;

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<String>,
}

fn proc_pattern_wrapper(request: FilterRequest, history: &[String]) -> GarminHtmlRequest {
    let filter = request.filter.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(ToString::to_string).collect();

    let req = GarminCli::process_pattern(&filter_vec);

    GarminHtmlRequest(GarminRequest {
        filter,
        history: history.to_vec(),
        ..req
    })
}

fn form_http_response(body: String) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json<T>(js: &T) -> Result<HttpResponse, Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json2(js))
}

pub fn garmin(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    let grec = proc_pattern_wrapper(query, &state.history.read());
    let history_size = state.history.read().len();
    if history_size > 5 {
        state.history.write().remove(0);
    }
    state.history.write().push(grec.0.filter.clone());

    state
        .db
        .send(grec)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

fn save_file(file_path: String, field: Field) -> impl Future<Item = i64, Error = Error> {
    let file = match File::create(file_path) {
        Ok(file) => file,
        Err(e) => return Either::A(err(format_err!("{:?}", e))),
    };
    Either::B(
        field
            .fold((file, 0i64), move |(mut file, mut acc), bytes| {
                // fs operations are blocking, we have to execute writes
                // on threadpool
                block(move || {
                    file.write_all(bytes.as_ref()).map_err(|e| {
                        MultipartError::Payload(actix_web::error::PayloadError::Io(e))
                    })?;
                    acc += bytes.len() as i64;
                    Ok((file, acc))
                })
                .map_err(|e: actix_web::error::BlockingError<MultipartError>| {
                    match e {
                        actix_web::error::BlockingError::Error(e) => e,
                        actix_web::error::BlockingError::Canceled => MultipartError::Incomplete,
                    }
                })
            })
            .map(|(_, acc)| acc)
            .map_err(|e| format_err!("{:?}", e)),
    )
}

pub fn garmin_upload(
    query: Query<GarminUploadRequest>,
    multipart: Multipart,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let tempdir = match TempDir::new("garmin") {
        Ok(d) => d,
        Err(e) => return Either::A(err(format_err!("{:?}", e))),
    };
    let tempdir_str = tempdir.path().to_string_lossy().to_string();

    let query = query.into_inner();
    let fname = format!("{}/{}", tempdir_str, query.filename);

    match multipart
        .map_err(err_msg)
        .map(move |field| save_file(fname.clone(), field).into_stream())
        .flatten()
        .collect()
        .wait()
    {
        Ok(_) => (),
        Err(e) => return Either::A(err(format_err!("{:?}", e))),
    };

    let fname = format!("{}/{}", tempdir_str, query.filename);
    Either::B(
        state
            .db
            .send(GarminUploadRequest { filename: fname })
            .from_err()
            .and_then(move |res| res.and_then(|flist| to_json(&flist))),
    )
}

pub fn garmin_connect_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(GarminConnectSyncRequest {})
        .from_err()
        .and_then(move |res| res.and_then(|flist| to_json(&flist)))
}

pub fn garmin_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(GarminSyncRequest {})
        .from_err()
        .and_then(move |res| {
            res.and_then(|body| {
                let body = format!(
                    r#"<textarea cols=100 rows=40>{}</textarea>"#,
                    body.join("\n")
                );
                form_http_response(body)
            })
        })
}

pub fn strava_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(StravaSyncRequest {})
        .from_err()
        .and_then(move |res| {
            res.and_then(|body| {
                let body = format!(
                    r#"<textarea cols=100 rows=40>{}</textarea>"#,
                    body.join("\n")
                );
                form_http_response(body)
            })
        })
}

pub fn strava_auth(
    query: Query<StravaAuthRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn strava_callback(
    query: Query<StravaCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn strava_activities(
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|alist| to_json(&alist)))
}

pub fn strava_upload(
    payload: Json<StravaUploadRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let payload = payload.into_inner();
    state
        .db
        .send(payload)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn strava_update(
    payload: Json<StravaUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let payload = payload.into_inner();
    state
        .db
        .send(payload)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn fitbit_auth(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(FitbitAuthRequest {})
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn fitbit_heartrate_api(
    query: Query<FitbitHeartrateApiRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|hlist| to_json(&hlist)))
}

pub fn fitbit_bodyweight(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(FitbitBodyWeightFatRequest {})
        .from_err()
        .and_then(move |res| res.and_then(|hlist| to_json(&hlist)))
}

pub fn fitbit_bodyweight_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(FitbitBodyWeightFatUpdateRequest {})
        .from_err()
        .and_then(move |res| res.and_then(|hlist| to_json(&hlist)))
}

pub fn fitbit_heartrate_db(
    query: Query<FitbitHeartrateDbRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|hlist| to_json(&hlist)))
}

pub fn fitbit_callback(
    query: Query<FitbitCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(form_http_response))
}

pub fn fitbit_sync(
    query: Query<FitbitSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|_| form_http_response("finished".into())))
}

pub fn fitbit_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();
    state.db.send(query).from_err().and_then(move |res| {
        res.and_then(|body| {
            let body = body.replace(
                "HISTORYBUTTONS",
                &generate_history_buttons(&state.history.read()),
            );
            form_http_response(body)
        })
    })
}

pub fn heartrate_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query: FitbitHeartratePlotRequest = query.into_inner().into();
    state.db.send(query).from_err().and_then(move |res| {
        res.and_then(|body| {
            let body = body.replace(
                "HISTORYBUTTONS",
                &generate_history_buttons(&state.history.read()),
            );
            form_http_response(body)
        })
    })
}

pub fn scale_measurement(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();
    state
        .db
        .send(query)
        .from_err()
        .and_then(move |res| res.and_then(|slist| to_json(&slist)))
}

pub fn scale_measurement_update(
    data: Json<ScaleMeasurementUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let measurements = data.into_inner();
    state
        .db
        .send(measurements)
        .from_err()
        .and_then(move |res| res.and_then(|_| form_http_response("finished".into())))
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
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    state.db.send(greq).from_err().and_then(move |res| {
        res.and_then(|gps_list| {
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
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    state
        .db
        .send(greq)
        .from_err()
        .join(state.db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| {
            res0.and_then(|file_list| {
                let hr_data = match file_list.len() {
                    1 => {
                        let config = &CONFIG;
                        let file_name = file_list
                            .get(0)
                            .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                        let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                        match GarminFile::read_avro(&avro_file) {
                            Ok(g) => g,
                            Err(_) => {
                                let gps_file = format!("{}/{}", &config.gps_dir, file_name);
                                let corr_map = res1.map(|c| c.corr_map)?;
                                GarminParse::new().with_file(&gps_file, &corr_map)?
                            }
                        }
                        .points
                        .iter()
                        .filter_map(|point| match point.heart_rate {
                            Some(heart_rate) => Some(TimeValue {
                                time: convert_datetime_to_str(point.time),
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
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    state
        .db
        .send(greq)
        .from_err()
        .join(state.db.send(GarminCorrRequest {}).from_err())
        .and_then(move |(res0, res1)| {
            res0.and_then(|file_list| {
                let hrpace = match file_list.len() {
                    1 => {
                        let config = &CONFIG;
                        let file_name = &file_list[0];
                        let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
                        let gfile = match GarminFile::read_avro(&avro_file) {
                            Ok(g) => g,
                            Err(_) => {
                                let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                                let corr_map = res1.map(|c| c.corr_map)?;

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
