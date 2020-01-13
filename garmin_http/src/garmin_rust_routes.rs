#![allow(clippy::needless_pass_by_value)]

use actix_multipart::{Field, Multipart};
use actix_web::http::StatusCode;
use actix_web::web::{block, Data, Json, Query};
use actix_web::HttpResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::string::ToString;
use tempdir::TempDir;
use tokio::stream::StreamExt;

use garmin_lib::common::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use garmin_lib::reports::garmin_file_report_html::generate_history_buttons;
use garmin_lib::reports::garmin_file_report_txt::get_splits;
use garmin_lib::utils::iso_8601_datetime::convert_datetime_to_str;

use super::errors::ServiceError as Error;
use super::logged_user::LoggedUser;

use super::garmin_rust_app::AppState;
use crate::garmin_requests::{
    FitbitAuthRequest, FitbitBodyWeightFatRequest, FitbitBodyWeightFatUpdateRequest,
    FitbitCallbackRequest, FitbitHeartrateApiRequest, FitbitHeartrateCacheRequest,
    FitbitHeartrateDbRequest, FitbitHeartratePlotRequest, FitbitSyncRequest, FitbitTcxSyncRequest,
    GarminConnectSyncRequest, GarminCorrRequest, GarminHtmlRequest, GarminListRequest,
    GarminSyncRequest, GarminUploadRequest, HandleRequest, ScaleMeasurementPlotRequest,
    ScaleMeasurementRequest, ScaleMeasurementUpdateRequest, StravaActiviesDBUpdateRequest,
    StravaActivitiesDBRequest, StravaActivitiesRequest, StravaAuthRequest, StravaCallbackRequest,
    StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
};
use crate::CONFIG;

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<String>,
}

fn proc_pattern_wrapper(request: FilterRequest, history: &[String]) -> GarminHtmlRequest {
    let filter = request.filter.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(ToString::to_string).collect();

    let req = GarminCli::process_pattern(&CONFIG, &filter_vec);

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

pub async fn garmin(
    query: Query<FilterRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let grec = proc_pattern_wrapper(query, &state.history.read());
    let history_size = state.history.read().len();
    if history_size > 5 {
        state.history.write().remove(0);
    }
    state.history.write().push(grec.0.filter.clone());

    let body = block(move || state.db.handle(grec)).await?;

    form_http_response(body)
}

async fn save_file(file_path: String, mut field: Field) -> Result<(), Error> {
    let mut file = block(|| File::create(file_path)).await?;

    while let Some(chunk) = field.next().await {
        let data = chunk?;
        file = block(move || file.write_all(&data).map(|_| file)).await?;
    }
    Ok(())
}

pub async fn garmin_upload(
    mut multipart: Multipart,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let tempdir = TempDir::new("garmin")?;
    let tempdir_str = tempdir.path().to_string_lossy().to_string();

    let fname = format!(
        "{}/{}",
        tempdir_str,
        Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string()
    );

    while let Some(item) = multipart.next().await {
        let field = item?;
        save_file(fname.clone(), field).await?;
    }

    let flist = block(move || state.db.handle(GarminUploadRequest { filename: fname })).await?;
    to_json(&flist)
}

pub async fn garmin_connect_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let flist = block(move || state.db.handle(GarminConnectSyncRequest {})).await?;
    to_json(&flist)
}

pub async fn garmin_sync(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = block(move || state.db.handle(GarminSyncRequest {})).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    form_http_response(body)
}

pub async fn strava_sync(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = block(move || state.db.handle(StravaSyncRequest {})).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    form_http_response(body)
}

pub async fn strava_auth(
    query: Query<StravaAuthRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let body = block(move || state.db.handle(query)).await?;
    form_http_response(body)
}

pub async fn strava_callback(
    query: Query<StravaCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let body = block(move || state.db.handle(query)).await?;
    form_http_response(body)
}

pub async fn strava_activities(
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let alist = block(move || state.db.handle(query)).await?;
    to_json(&alist)
}

pub async fn strava_activities_db(
    query: Query<StravaActivitiesRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = StravaActivitiesDBRequest(query.into_inner());
    let alist = block(move || state.db.handle(query)).await?;
    to_json(&alist)
}

pub async fn strava_activities_db_update(
    payload: Json<StravaActiviesDBUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = payload.into_inner();
    let body = block(move || state.db.handle(req)).await?;
    form_http_response(body.join("\n"))
}

pub async fn strava_upload(
    payload: Json<StravaUploadRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payload = payload.into_inner();
    let body = block(move || state.db.handle(payload)).await?;
    form_http_response(body)
}

pub async fn strava_update(
    payload: Json<StravaUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payload = payload.into_inner();
    let body = block(move || state.db.handle(payload)).await?;
    form_http_response(body)
}

pub async fn fitbit_auth(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = FitbitAuthRequest {};
    let body = block(move || state.db.handle(req)).await?;
    form_http_response(body)
}

pub async fn fitbit_heartrate_api(
    query: Query<FitbitHeartrateApiRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = block(move || state.db.handle(query)).await?;
    to_json(&hlist)
}

pub async fn fitbit_heartrate_cache(
    query: Query<FitbitHeartrateCacheRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = block(move || state.db.handle(query)).await?;
    to_json(&hlist)
}

pub async fn fitbit_bodyweight(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = FitbitBodyWeightFatRequest {};
    let hlist = block(move || state.db.handle(req)).await?;
    to_json(&hlist)
}

pub async fn fitbit_bodyweight_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = FitbitBodyWeightFatUpdateRequest {};
    let hlist = block(move || state.db.handle(req)).await?;
    to_json(&hlist)
}

pub async fn fitbit_heartrate_db(
    query: Query<FitbitHeartrateDbRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = block(move || state.db.handle(query)).await?;
    to_json(&hlist)
}

pub async fn fitbit_callback(
    query: Query<FitbitCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let body = block(move || state.db.handle(query)).await?;
    form_http_response(body)
}

pub async fn fitbit_sync(
    query: Query<FitbitSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    block(move || state.db.handle(query)).await?;
    form_http_response("finished".into())
}

pub async fn fitbit_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();
    let s = state.clone();
    let body = block(move || s.db.handle(query)).await?;

    let body = body.replace(
        "HISTORYBUTTONS",
        &generate_history_buttons(&state.history.read()),
    );
    form_http_response(body)
}

pub async fn heartrate_plots(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query: FitbitHeartratePlotRequest = query.into_inner().into();
    let s = state.clone();
    let body = block(move || s.db.handle(query)).await?;
    let body = body.replace(
        "HISTORYBUTTONS",
        &generate_history_buttons(&state.history.read()),
    );
    form_http_response(body)
}

pub async fn fitbit_tcx_sync(
    query: Query<FitbitTcxSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let flist = block(move || state.db.handle(query)).await?;
    to_json(&flist)
}

pub async fn scale_measurement(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let slist = block(move || state.db.handle(query)).await?;
    to_json(&slist)
}

pub async fn scale_measurement_update(
    data: Json<ScaleMeasurementUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let measurements = data.into_inner();
    block(move || state.db.handle(measurements)).await?;
    form_http_response("finished".into())
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

pub async fn garmin_list_gps_tracks(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    let gps_list = block(move || state.db.handle(greq)).await?;
    let glist = GpsList { gps_list };
    to_json(&glist)
}

#[derive(Serialize)]
pub struct HrData {
    pub hr_data: Vec<TimeValue>,
}

pub async fn garmin_get_hr_data(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    let s = state.clone();
    let file_list = block(move || s.db.handle(greq)).await?;

    let hr_data = match file_list.len() {
        1 => {
            let config = &CONFIG;
            let file_name = &file_list[0];
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let a = avro_file.clone();
            match block(move || GarminFile::read_avro(&a)).await {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);
                    let corr_map = block(move || state.db.handle(GarminCorrRequest {}))
                        .await?
                        .corr_map;
                    let gfile =
                        block(move || GarminParse::new().with_file(&gps_file, &corr_map)).await?;
                    block(move || gfile.dump_avro(&avro_file).map(|_| gfile)).await?
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

pub async fn garmin_get_hr_pace(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();

    let greq: GarminListRequest = proc_pattern_wrapper(query, &state.history.read()).into();

    let s = state.clone();
    let file_list = block(move || s.db.handle(greq)).await?;

    let hrpace = match file_list.len() {
        1 => {
            let config = &CONFIG;
            let file_name = &file_list[0];
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let gfile = match block(move || GarminFile::read_avro(&avro_file)).await {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                    let corr_map = block(move || state.db.handle(GarminCorrRequest {}))
                        .await?
                        .corr_map;

                    block(move || GarminParse::new().with_file(&gps_file, &corr_map)).await?
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
}
