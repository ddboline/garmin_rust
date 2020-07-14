#![allow(clippy::needless_pass_by_value)]

use actix_multipart::{Field, Multipart};
use actix_session::Session;
use actix_web::{
    http::StatusCode,
    web::{Data, Json, Query},
    HttpResponse,
};
use anyhow::format_err;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::string::ToString;
use tempdir::TempDir;
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt, task::spawn_blocking};

use garmin_lib::{
    common::{
        garmin_cli::{GarminCli, GarminRequest},
        garmin_file::GarminFile,
    },
    parsers::garmin_parse::{GarminParse, GarminParseTrait},
    reports::{
        garmin_file_report_html::generate_history_buttons, garmin_file_report_txt::get_splits,
    },
    utils::{iso_8601_datetime::convert_datetime_to_str, stack_string::StackString},
};

use super::{errors::ServiceError as Error, logged_user::LoggedUser};

use super::garmin_rust_app::AppState;
use crate::{
    garmin_requests::{
        AddGarminCorrectionRequest, FitbitActivitiesDBRequest, FitbitActivitiesDBUpdateRequest,
        FitbitActivitiesRequest, FitbitActivityTypesRequest, FitbitAuthRequest,
        FitbitBodyWeightFatRequest, FitbitBodyWeightFatUpdateRequest, FitbitCallbackRequest,
        FitbitHeartrateApiRequest, FitbitHeartrateCacheRequest, FitbitHeartratePlotRequest,
        FitbitProfileRequest, FitbitRefreshRequest, FitbitStatisticsPlotRequest, FitbitSyncRequest,
        FitbitTcxSyncRequest, GarminConnectActivitiesDBRequest,
        GarminConnectActivitiesDBUpdateRequest, GarminConnectActivitiesRequest,
        GarminConnectHrApiRequest, GarminConnectHrSyncRequest, GarminConnectSyncRequest,
        GarminCorrRequest, GarminHtmlRequest, GarminListRequest, GarminSyncRequest,
        GarminUploadRequest, HandleRequest, ScaleMeasurementPlotRequest, ScaleMeasurementRequest,
        ScaleMeasurementUpdateRequest, StravaActiviesDBUpdateRequest, StravaActivitiesDBRequest,
        StravaActivitiesRequest, StravaAthleteRequest, StravaAuthRequest, StravaCallbackRequest,
        StravaRefreshRequest, StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
    },
    CONFIG,
};

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<StackString>,
}

fn proc_pattern_wrapper<T: AsRef<str>>(
    request: FilterRequest,
    history: &[T],
    is_demo: bool,
) -> GarminHtmlRequest {
    let filter = request
        .filter
        .as_ref()
        .map_or_else(|| "sport", StackString::as_str);

    let filter_vec: Vec<_> = filter.split(',').map(ToString::to_string).collect();

    let req = GarminCli::process_pattern(&CONFIG, &filter_vec);
    let history: Vec<_> = history.iter().map(|s| s.as_ref().into()).collect();

    GarminHtmlRequest {
        request: GarminRequest {
            filter: filter.into(),
            history,
            ..req
        },
        is_demo,
    }
}

fn form_http_response(body: String) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json<T>(js: T) -> Result<HttpResponse, Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json(js))
}

pub async fn garmin(
    query: Query<FilterRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let mut history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);
    let grec = proc_pattern_wrapper(query, &history, false);
    if history.len() > 5 {
        history.remove(0);
    }
    history.push(grec.request.filter.as_str().into());
    session
        .set("history", history)
        .map_err(|e| format_err!("Failed to set history {:?}", e))?;

    let body = state.db.handle(grec).await?;

    form_http_response(body.into())
}

pub async fn garmin_demo(
    query: Query<FilterRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let mut history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);
    let grec = proc_pattern_wrapper(query, &history, true);
    if history.len() > 5 {
        history.remove(0);
    }
    history.push(grec.request.filter.as_ref().into());
    session
        .set("history", history)
        .map_err(|e| format_err!("Failed to set history {:?}", e))?;

    let body = state.db.handle(grec).await?;

    form_http_response(body.into())
}

async fn save_file(file_path: &str, mut field: Field) -> Result<(), Error> {
    let mut file = File::create(file_path).await?;

    while let Some(chunk) = field.next().await {
        let data = chunk?;
        file.write_all(&data).await?;
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
        save_file(&fname, field).await?;
    }

    let flist = state
        .db
        .handle(GarminUploadRequest {
            filename: fname.into(),
        })
        .await?;
    to_json(flist)
}

pub async fn garmin_connect_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let flist = state.db.handle(GarminConnectSyncRequest {}).await?;
    to_json(flist)
}

pub async fn garmin_connect_hr_sync(
    query: Query<GarminConnectHrSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    state.db.handle(query).await?;
    form_http_response("".to_string())
}

pub async fn garmin_connect_hr_api(
    query: Query<GarminConnectHrApiRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hr_vals = state.db.handle(query).await?;
    to_json(hr_vals)
}

pub async fn garmin_sync(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = state.db.handle(GarminSyncRequest {}).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    form_http_response(body)
}

pub async fn strava_sync(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = state.db.handle(StravaSyncRequest {}).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    form_http_response(body)
}

pub async fn strava_auth(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = state.db.handle(StravaAuthRequest {}).await?;
    form_http_response(body.into())
}

pub async fn strava_refresh(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let body = state.db.handle(StravaRefreshRequest {}).await?;
    form_http_response(body.into())
}

pub async fn strava_callback(
    query: Query<StravaCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let body = state.db.handle(query).await?;
    form_http_response(body.into())
}

pub async fn strava_activities(
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let alist = state.db.handle(query).await?;
    to_json(alist)
}

pub async fn strava_activities_db(
    query: Query<StravaActivitiesRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = StravaActivitiesDBRequest(query.into_inner());
    let alist = state.db.handle(query).await?;
    to_json(alist)
}

pub async fn strava_activities_db_update(
    payload: Json<StravaActiviesDBUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = payload.into_inner();
    let body = state.db.handle(req).await?;
    form_http_response(body.join("\n"))
}

pub async fn strava_upload(
    payload: Json<StravaUploadRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payload = payload.into_inner();
    let body = state.db.handle(payload).await?;
    form_http_response(body.into())
}

pub async fn strava_update(
    payload: Json<StravaUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payload = payload.into_inner();
    let body = state.db.handle(payload).await?;
    form_http_response(body.into())
}

pub async fn fitbit_auth(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = FitbitAuthRequest {};
    let body = state.db.handle(req).await?;
    form_http_response(body.into())
}

pub async fn fitbit_refresh(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = FitbitRefreshRequest {};
    let body = state.db.handle(req).await?;
    form_http_response(body.into())
}

pub async fn fitbit_heartrate_api(
    query: Query<FitbitHeartrateApiRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = state.db.handle(query).await?;
    to_json(hlist)
}

pub async fn fitbit_heartrate_cache(
    query: Query<FitbitHeartrateCacheRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = state.db.handle(query).await?;
    to_json(hlist)
}

pub async fn fitbit_bodyweight(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = FitbitBodyWeightFatRequest {};
    let hlist = state.db.handle(req).await?;
    to_json(hlist)
}

pub async fn fitbit_bodyweight_sync(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = FitbitBodyWeightFatUpdateRequest {};
    let hlist = state.db.handle(req).await?;
    to_json(hlist)
}

pub async fn fitbit_activities(
    query: Query<FitbitActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let hlist = state.db.handle(query).await?;
    to_json(hlist)
}

pub async fn fitbit_callback(
    query: Query<FitbitCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let body = state.db.handle(query).await?;
    form_http_response(body.into())
}

pub async fn fitbit_sync(
    query: Query<FitbitSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    state.db.handle(query).await?;
    form_http_response("finished".into())
}

pub async fn heartrate_statistics_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query: FitbitStatisticsPlotRequest = query.into_inner().into();

    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let body = state
        .db
        .handle(query)
        .await?
        .replace("HISTORYBUTTONS", &generate_history_buttons(&history));
    form_http_response(body)
}

async fn fitbit_plots_impl(
    query: ScaleMeasurementPlotRequest,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let body = state
        .db
        .handle(query)
        .await?
        .replace("HISTORYBUTTONS", &generate_history_buttons(&history));
    form_http_response(body)
}

pub async fn fitbit_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();
    fitbit_plots_impl(query, state, session).await
}

pub async fn fitbit_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let mut query: ScaleMeasurementPlotRequest = query.into_inner().into();
    query.is_demo = true;
    fitbit_plots_impl(query, state, session).await
}

async fn heartrate_plots_impl(
    query: FitbitHeartratePlotRequest,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let body = state
        .db
        .handle(query)
        .await?
        .replace("HISTORYBUTTONS", &generate_history_buttons(&history));
    form_http_response(body)
}

pub async fn heartrate_plots(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query: FitbitHeartratePlotRequest = query.into_inner().into();
    heartrate_plots_impl(query, state, session).await
}

pub async fn heartrate_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let mut query: FitbitHeartratePlotRequest = query.into_inner().into();
    query.is_demo = true;
    heartrate_plots_impl(query, state, session).await
}

pub async fn fitbit_tcx_sync(
    query: Query<FitbitTcxSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let flist = state.db.handle(query).await?;
    to_json(flist)
}

pub async fn scale_measurement(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let slist = state.db.handle(query).await?;
    to_json(slist)
}

pub async fn scale_measurement_update(
    data: Json<ScaleMeasurementUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let measurements = data.into_inner();
    state.db.handle(measurements).await?;
    form_http_response("finished".into())
}

#[derive(Serialize)]
pub struct GpsList {
    pub gps_list: Vec<StackString>,
}

#[derive(Serialize)]
pub struct TimeValue {
    pub time: StackString,
    pub value: f64,
}

pub async fn garmin_list_gps_tracks(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let greq: GarminListRequest = proc_pattern_wrapper(query, &history, false).into();
    let gps_list: Vec<_> = state
        .db
        .handle(greq)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    let glist = GpsList { gps_list };
    to_json(glist)
}

#[derive(Serialize)]
pub struct HrData {
    pub hr_data: Vec<TimeValue>,
}

pub async fn garmin_get_hr_data(
    query: Query<FilterRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let greq: GarminListRequest = proc_pattern_wrapper(query, &history, false).into();

    let s = state.clone();
    let file_list = s.db.handle(greq).await?;

    let hr_data = match file_list.len() {
        1 => {
            let config = &CONFIG;
            let file_name = &file_list[0];
            let avro_file = config.cache_dir.join(&format!("{}.avro", file_name));
            let a = avro_file.clone();

            if let Ok(g) = spawn_blocking(move || GarminFile::read_avro(&a)).await? {
                g
            } else {
                let gps_file = config.gps_dir.join(file_name.as_str());
                let corr_map = state.db.handle(GarminCorrRequest {}).await?;
                let gfile =
                    spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map))
                        .await??;
                spawn_blocking(move || gfile.dump_avro(&avro_file).map(|_| gfile)).await??
            }
            .points
            .iter()
            .filter_map(|point| match point.heart_rate {
                Some(heart_rate) => Some(TimeValue {
                    time: convert_datetime_to_str(point.time).into(),
                    value: heart_rate,
                }),
                None => None,
            })
            .collect()
        }
        _ => Vec::new(),
    };
    let hdata = HrData { hr_data };
    to_json(hdata)
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
    session: Session,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let greq: GarminListRequest = proc_pattern_wrapper(query, &history, false).into();

    let s = state.clone();
    let file_list = s.db.handle(greq).await?;

    let hrpace = match file_list.len() {
        1 => {
            let config = &CONFIG;
            let file_name = &file_list[0];
            let avro_file = config.cache_dir.join(&format!("{}.avro", file_name));

            let gfile = if let Ok(g) =
                spawn_blocking(move || GarminFile::read_avro(&avro_file)).await?
            {
                g
            } else {
                let gps_file = config.gps_dir.join(file_name.as_str());

                let corr_map = state.db.handle(GarminCorrRequest {}).await?;

                spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map)).await??
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
    to_json(hrpace)
}

pub async fn user(user: LoggedUser) -> Result<HttpResponse, Error> {
    to_json(user)
}

pub async fn add_garmin_correction(
    payload: Json<AddGarminCorrectionRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payload = payload.into_inner();
    state.db.handle(payload).await?;
    form_http_response("finised".to_string())
}

pub async fn fitbit_activity_types(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let result = state.db.handle(FitbitActivityTypesRequest {}).await?;
    to_json(result)
}

pub async fn strava_athlete(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let result = state.db.handle(StravaAthleteRequest {}).await?;
    to_json(result)
}

pub async fn fitbit_profile(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let result = state.db.handle(FitbitProfileRequest {}).await?;
    to_json(result)
}

pub async fn garmin_connect_activities(
    query: Query<GarminConnectActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let result = state.db.handle(query).await?;
    to_json(result)
}

pub async fn garmin_connect_activities_db(
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let alist = state
        .db
        .handle(GarminConnectActivitiesDBRequest(query))
        .await?;
    to_json(alist)
}

pub async fn garmin_connect_activities_db_update(
    payload: Json<GarminConnectActivitiesDBUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = payload.into_inner();
    let body = state.db.handle(req).await?;
    form_http_response(body.join("\n"))
}

pub async fn fitbit_activities_db(
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let alist = state.db.handle(FitbitActivitiesDBRequest(query)).await?;
    to_json(alist)
}

pub async fn fitbit_activities_db_update(
    payload: Json<FitbitActivitiesDBUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = payload.into_inner();
    let body = state.db.handle(req).await?;
    form_http_response(body.join("\n"))
}
