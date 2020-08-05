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
use stack_string::StackString;
use std::string::ToString;
use tempdir::TempDir;
use tokio::{fs::File, io::AsyncWriteExt, stream::StreamExt};

use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{
    common::garmin_templates::HBR, utils::iso_8601_datetime::convert_datetime_to_str,
};
use garmin_reports::garmin_file_report_html::generate_history_buttons;

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
        GarminConnectUserSummaryRequest, GarminHtmlRequest, GarminSyncRequest, GarminUploadRequest,
        HandleRequest, RaceResultFlagRequest, RaceResultImportRequest, RaceResultPlotRequest,
        ScaleMeasurementPlotRequest, ScaleMeasurementRequest, ScaleMeasurementUpdateRequest,
        StravaActiviesDBUpdateRequest, StravaActivitiesDBRequest, StravaActivitiesRequest,
        StravaAthleteRequest, StravaAuthRequest, StravaCallbackRequest, StravaCreateRequest,
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
    history.push(grec.request.filter.clone());
    session
        .set("history", history)
        .map_err(|e| format_err!("Failed to set history {:?}", e))?;

    let body = state.db.handle(grec).await?;

    form_http_response(body.into())
}

async fn save_file(file_path: &str, field: &mut Field) -> Result<(), Error> {
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
    session: Session,
) -> Result<HttpResponse, Error> {
    let tempdir = TempDir::new("garmin")?;
    let tempdir_str = tempdir.path().to_string_lossy().to_string();

    let fname = format!(
        "{}/{}",
        tempdir_str,
        Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string()
    );

    while let Some(item) = multipart.next().await {
        let mut field = item?;
        save_file(&fname, &mut field).await?;
    }

    let datetimes = state
        .db
        .handle(GarminUploadRequest {
            filename: fname.into(),
        })
        .await?;

    let query = FilterRequest {
        filter: datetimes
            .get(0)
            .map(|dt| convert_datetime_to_str(*dt).into()),
    };
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let grec = proc_pattern_wrapper(query, &history, false);
    let body = state.db.handle(grec).await?;

    form_http_response(body.into())
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
    let heartrates = state.db.handle(query).await?;
    form_http_response(heartrates.to_table().into())
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

pub async fn strava_create(
    query: Query<StravaCreateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let activity_id = state.db.handle(query).await?;
    if let Some(activity_id) = activity_id {
        form_http_response(activity_id.to_string())
    } else {
        form_http_response("".into())
    }
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
    let heartrates = state.db.handle(query).await?;
    form_http_response(FitbitHeartRate::create_table(&heartrates).into())
}

pub async fn heartrate_statistics_plots_impl(
    query: FitbitStatisticsPlotRequest,
    state: Data<AppState>,
    session: Session,
) -> Result<StackString, Error> {
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);

    let template = if query.is_demo {
        "PLOT_TEMPLATE_DEMO"
    } else {
        "PLOT_TEMPLATE"
    };
    let mut params = state.db.handle(query).await?;
    params.insert("HISTORYBUTTONS".into(), generate_history_buttons(&history));
    Ok(HBR.render(template, &params)?.into())
}

pub async fn heartrate_statistics_plots(
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let query: FitbitStatisticsPlotRequest = query.into_inner().into();

    let body = heartrate_statistics_plots_impl(query, state, session).await?;
    form_http_response(body.into())
}

pub async fn heartrate_statistics_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let mut query: FitbitStatisticsPlotRequest = query.into_inner().into();
    query.is_demo = true;

    let body = heartrate_statistics_plots_impl(query, state, session).await?;
    form_http_response(body.into())
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
    let template = if query.is_demo {
        "PLOT_TEMPLATE_DEMO"
    } else {
        "PLOT_TEMPLATE"
    };
    let mut params = state.db.handle(query).await?;
    params.insert("HISTORYBUTTONS".into(), generate_history_buttons(&history));
    let body = HBR.render(template, &params)?;
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
    let template = if query.is_demo {
        "PLOT_TEMPLATE_DEMO"
    } else {
        "PLOT_TEMPLATE"
    };
    let mut params = state.db.handle(query).await?;
    params.insert("HISTORYBUTTONS".into(), generate_history_buttons(&history));
    let body = HBR.render(template, &params)?;
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

#[derive(Serialize)]
pub struct HrData {
    pub hr_data: Vec<TimeValue>,
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

pub async fn garmin_connect_user_summary(
    query: Query<GarminConnectUserSummaryRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let js = state.db.handle(query).await?;
    to_json(js)
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

pub async fn race_result_plot_impl(
    req: RaceResultPlotRequest,
    state: Data<AppState>,
    session: Session,
) -> Result<StackString, Error> {
    let history: Vec<StackString> = session
        .get("history")
        .map_err(|e| format_err!("Failed to set history {:?}", e))?
        .unwrap_or_else(Vec::new);
    let is_demo = req.demo.unwrap_or(true);
    let template = if is_demo {
        "PLOT_TEMPLATE_DEMO"
    } else {
        "PLOT_TEMPLATE"
    };
    let mut params = state.db.handle(req).await?;
    params.insert("HISTORYBUTTONS".into(), generate_history_buttons(&history));
    Ok(HBR.render(template, &params)?.into())
}

pub async fn race_result_plot(
    query: Query<RaceResultPlotRequest>,
    _: LoggedUser,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let mut query = query.into_inner();
    query.demo = Some(false);

    let body = race_result_plot_impl(query, state, session).await?;
    form_http_response(body.into())
}

pub async fn race_result_plot_demo(
    query: Query<RaceResultPlotRequest>,
    state: Data<AppState>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let mut query = query.into_inner();
    query.demo = Some(true);

    let body = race_result_plot_impl(query, state, session).await?;
    form_http_response(body.into())
}

pub async fn race_result_flag(
    query: Query<RaceResultFlagRequest>,
    state: Data<AppState>,
    _: LoggedUser,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    let result = state.db.handle(query).await?;
    form_http_response(result.into())
}

pub async fn race_result_import(
    query: Query<RaceResultImportRequest>,
    state: Data<AppState>,
    _: LoggedUser,
) -> Result<HttpResponse, Error> {
    let query = query.into_inner();
    state.db.handle(query).await?;
    form_http_response("".into())
}
