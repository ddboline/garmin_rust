#![allow(clippy::needless_pass_by_value)]

use http::header::SET_COOKIE;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{str::FromStr, string::ToString};
use tempdir::TempDir;
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_stream::StreamExt;
use warp::{
    multipart::{FormData, Part},
    Buf, Rejection, Reply,
};

use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{
    common::{
        garmin_config::GarminConfig,
        garmin_templates::{get_buttons, get_scripts, get_style, HBR},
    },
    utils::iso_8601_datetime::convert_datetime_to_str,
};
use garmin_reports::garmin_file_report_html::generate_history_buttons;

use crate::{
    errors::ServiceError as Error,
    garmin_requests::{
        AddGarminCorrectionRequest, FitbitActivitiesDBRequest, FitbitActivitiesDBUpdateRequest,
        FitbitActivitiesRequest, FitbitActivityTypesRequest, FitbitAuthRequest,
        FitbitBodyWeightFatRequest, FitbitBodyWeightFatUpdateRequest, FitbitCallbackRequest,
        FitbitHeartrateApiRequest, FitbitHeartrateCacheRequest, FitbitHeartratePlotRequest,
        FitbitHeartrateUpdateRequest, FitbitProfileRequest, FitbitRefreshRequest,
        FitbitStatisticsPlotRequest, FitbitSyncRequest, FitbitTcxSyncRequest,
        GarminConnectActivitiesDBRequest, GarminConnectActivitiesDBUpdateRequest,
        GarminConnectActivitiesRequest, GarminConnectHrApiRequest, GarminConnectHrSyncRequest,
        GarminConnectSyncRequest, GarminConnectUserSummaryRequest, GarminHtmlRequest,
        GarminSyncRequest, GarminUploadRequest, HeartrateStatisticsSummaryDBRequest,
        HeartrateStatisticsSummaryDBUpdateRequest, RaceResultFlagRequest, RaceResultImportRequest,
        RaceResultPlotRequest, RaceResultsDBRequest, RaceResultsDBUpdateRequest,
        ScaleMeasurementPlotRequest, ScaleMeasurementRequest, ScaleMeasurementUpdateRequest,
        StravaActiviesDBUpdateRequest, StravaActivitiesDBRequest, StravaActivitiesRequest,
        StravaAthleteRequest, StravaAuthRequest, StravaCallbackRequest, StravaCreateRequest,
        StravaRefreshRequest, StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
    },
    garmin_rust_app::AppState,
    logged_user::LoggedUser,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

#[derive(Default)]
pub struct Session {
    history: Vec<StackString>,
}

impl FromStr for Session {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = base64::decode(s)?;
        let history_str = String::from_utf8(data)?;
        let history = history_str.split(',').map(Into::into).collect();
        Ok(Session { history })
    }
}

impl Session {
    pub fn get_jwt_cookie(&self, domain: &str) -> String {
        let history_str = self.history.join(",");
        let token = base64::encode(history_str);
        format!("jwt={}; HttpOnly; Path=/; Domain={}", token, domain)
    }
}

#[derive(Deserialize)]
pub struct FilterRequest {
    pub filter: Option<StackString>,
}

fn proc_pattern_wrapper<T: AsRef<str>>(
    config: &GarminConfig,
    request: FilterRequest,
    history: &[T],
    is_demo: bool,
) -> GarminHtmlRequest {
    let filter = request
        .filter
        .as_ref()
        .map_or_else(|| "sport", StackString::as_str);

    let filter_iter = filter.split(',').map(ToString::to_string);

    let req = GarminCli::process_pattern(&config, filter_iter);
    let history = history.iter().map(|s| s.as_ref().into()).collect();

    GarminHtmlRequest {
        request: GarminRequest {
            filter: filter.into(),
            history,
            ..req
        },
        is_demo,
    }
}

pub async fn garmin(
    query: FilterRequest,
    _: LoggedUser,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let mut session = session.unwrap_or_default();
    let body = garmin_body(query, &state, &mut session.history, false).await?;
    let jwt = session.get_jwt_cookie(&state.config.domain);
    let reply = warp::reply::html(body);
    let reply = warp::reply::with_header(reply, SET_COOKIE, jwt);
    Ok(reply)
}

async fn garmin_body(
    query: FilterRequest,
    state: &AppState,
    history: &mut Vec<StackString>,
    is_demo: bool,
) -> HttpResult<String> {
    let grec = proc_pattern_wrapper(&state.config, query, &history, is_demo);
    if history.len() > 5 {
        history.remove(0);
    }
    history.push(grec.request.filter.as_str().into());
    let body = grec.handle(&state.db).await?;

    Ok(body.into())
}

pub async fn garmin_demo(
    query: FilterRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let mut session = session.unwrap_or_default();
    let body = garmin_body(query, &state, &mut session.history, true).await?;
    let jwt = session.get_jwt_cookie(&state.config.domain);
    let reply = warp::reply::html(body);
    let reply = warp::reply::with_header(reply, SET_COOKIE, jwt);
    Ok(reply)
}

pub async fn garmin_upload(
    query: StravaCreateRequest,
    form: FormData,
    _: LoggedUser,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let session = session.unwrap_or_default();
    let body = garmin_upload_body(query, form, state, session).await?;
    Ok(warp::reply::html(body))
}

async fn garmin_upload_body(
    query: StravaCreateRequest,
    mut form: FormData,
    state: AppState,
    session: Session,
) -> HttpResult<String> {
    let tempdir = TempDir::new("garmin")?;
    let tempdir_str = tempdir.path().to_string_lossy().to_string();

    let fname = format!("{}/{}", tempdir_str, query.filename,);

    while let Some(item) = form.next().await {
        save_file(&fname, item?).await?;
    }

    let datetimes = GarminUploadRequest {
        filename: fname.into(),
    }
    .handle(&state.db)
    .await?;

    let query = FilterRequest {
        filter: datetimes
            .get(0)
            .map(|dt| convert_datetime_to_str(*dt).into()),
    };

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    let body = grec.handle(&state.db).await?;

    Ok(body.into())
}

async fn save_file(file_path: &str, field: Part) -> Result<(), anyhow::Error> {
    let mut file = File::create(file_path).await?;
    let mut stream = field.stream();

    while let Some(chunk) = stream.next().await {
        file.write_all(chunk?.chunk()).await?;
    }
    Ok(())
}

pub async fn garmin_connect_sync(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body = GarminConnectSyncRequest {}
        .handle(&state.db, &state.connect_proxy)
        .await?;
    Ok(warp::reply::json(&body))
}

pub async fn garmin_connect_hr_sync(
    query: GarminConnectHrSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let heartrates = query
        .handle(&state.db, &state.connect_proxy, &state.config)
        .await?;
    let body: String = heartrates.to_table(Some(20)).into();
    Ok(warp::reply::html(body))
}

pub async fn garmin_connect_hr_api(
    query: GarminConnectHrApiRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let hr_vals = query.handle(state.connect_proxy).await?;
    Ok(warp::reply::json(&hr_vals))
}

pub async fn garmin_sync(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body = GarminSyncRequest {}.handle(&state.db).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    Ok(warp::reply::html(body))
}

pub async fn strava_sync(
    query: StravaSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body = query
        .handle(&state.db, &state.config)
        .await?
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .join("\n");
    let body = format!(r#"<textarea cols=100 rows=40>{}</textarea>"#, body);
    Ok(warp::reply::html(body))
}

pub async fn strava_auth(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body: String = StravaAuthRequest {}.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn strava_refresh(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body: String = StravaRefreshRequest {}.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn strava_callback(
    query: StravaCallbackRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body: String = query.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn strava_activities(
    query: StravaActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let alist = query.handle(&state.config).await?;
    Ok(warp::reply::json(&alist))
}

pub async fn strava_activities_db(
    query: StravaActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let alist = StravaActivitiesDBRequest(query).handle(&state.db).await?;
    Ok(warp::reply::json(&alist))
}

pub async fn strava_activities_db_update(
    payload: StravaActiviesDBUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body = payload.handle(&state.db).await?;
    let body = body.join("\n");
    Ok(warp::reply::html(body))
}

pub async fn strava_upload(
    payload: StravaUploadRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body: String = payload.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn strava_update(
    payload: StravaUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body: String = payload.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn strava_create(
    query: StravaCreateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let activity_id = query.handle(&state.db, &state.config).await?;
    let body = activity_id.map_or_else(|| "".into(), |activity_id| activity_id.to_string());
    Ok(warp::reply::html(body))
}

pub async fn fitbit_auth(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body: String = FitbitAuthRequest {}.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn fitbit_refresh(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let body: String = FitbitRefreshRequest {}.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn fitbit_heartrate_api(
    query: FitbitHeartrateApiRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let hlist = query.handle(&state.config).await?;
    Ok(warp::reply::json(&hlist))
}

pub async fn fitbit_heartrate_cache(
    query: FitbitHeartrateCacheRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let hlist = query.handle(&state.config).await?;
    Ok(warp::reply::json(&hlist))
}

pub async fn fitbit_heartrate_cache_update(
    payload: FitbitHeartrateUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    payload.handle(&state.config).await?;
    Ok(warp::reply::html(""))
}

pub async fn fitbit_bodyweight(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let hlist = FitbitBodyWeightFatRequest {}.handle(&state.config).await?;
    Ok(warp::reply::json(&hlist))
}

pub async fn fitbit_bodyweight_sync(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let hlist = FitbitBodyWeightFatUpdateRequest {}
        .handle(&state.db, &state.config)
        .await?;
    Ok(warp::reply::json(&hlist))
}

pub async fn fitbit_activities(
    query: FitbitActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let hlist = query.handle(&state.config).await?;
    Ok(warp::reply::json(&hlist))
}

pub async fn fitbit_callback(
    query: FitbitCallbackRequest,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body: String = query.handle(&state.config).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn fitbit_sync(
    query: FitbitSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let heartrates = query.handle(&state.db, &state.config).await?;
    let start = if heartrates.len() > 20 {
        heartrates.len() - 20
    } else {
        0
    };
    let body: String = FitbitHeartRate::create_table(&heartrates[start..]).into();
    Ok(warp::reply::html(body))
}

async fn heartrate_statistics_plots_impl(
    query: FitbitStatisticsPlotRequest,
    state: AppState,
    session: Session,
) -> Result<StackString, Error> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = query.handle(&state.db).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    Ok(HBR.render("GARMIN_TEMPLATE", &params)?.into())
}

pub async fn heartrate_statistics_plots(
    query: ScaleMeasurementRequest,
    _: LoggedUser,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let query: FitbitStatisticsPlotRequest = query.into();
    let session = session.unwrap_or_default();

    let body: String = heartrate_statistics_plots_impl(query, state, session)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

pub async fn heartrate_statistics_plots_demo(
    query: ScaleMeasurementRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let mut query: FitbitStatisticsPlotRequest = query.into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let body: String = heartrate_statistics_plots_impl(query, state, session)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

async fn fitbit_plots_impl(
    query: ScaleMeasurementPlotRequest,
    state: AppState,
    session: Session,
) -> HttpResult<String> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = query.handle(&state.db).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    let body = HBR.render("GARMIN_TEMPLATE", &params)?;
    Ok(body)
}

pub async fn fitbit_plots(
    query: ScaleMeasurementRequest,
    _: LoggedUser,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let session = session.unwrap_or_default();
    let query: ScaleMeasurementPlotRequest = query.into();
    fitbit_plots_impl(query, state, session)
        .await
        .map_err(Into::into)
}

pub async fn fitbit_plots_demo(
    query: ScaleMeasurementRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let session = session.unwrap_or_default();
    let mut query: ScaleMeasurementPlotRequest = query.into();
    query.is_demo = true;
    fitbit_plots_impl(query, state, session)
        .await
        .map_err(Into::into)
}

async fn heartrate_plots_impl(
    query: FitbitHeartratePlotRequest,
    state: AppState,
    session: Session,
) -> HttpResult<String> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = query.handle(&state.db, &state.config).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    let body = HBR.render("GARMIN_TEMPLATE", &params)?;
    Ok(body)
}

pub async fn heartrate_plots(
    query: ScaleMeasurementRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let query: FitbitHeartratePlotRequest = query.into();
    let session = session.unwrap_or_default();
    heartrate_plots_impl(query, state, session)
        .await
        .map_err(Into::into)
}

pub async fn heartrate_plots_demo(
    query: ScaleMeasurementRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    let mut query: FitbitHeartratePlotRequest = query.into();
    query.is_demo = true;
    let session = session.unwrap_or_default();
    heartrate_plots_impl(query, state, session)
        .await
        .map_err(Into::into)
}

pub async fn fitbit_tcx_sync(
    query: FitbitTcxSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let flist = query.handle(&state.db, &state.config).await?;
    Ok(warp::reply::json(&flist))
}

pub async fn scale_measurement(
    query: ScaleMeasurementRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let slist = query.handle(&state.db).await?;
    Ok(warp::reply::json(&slist))
}

pub async fn scale_measurement_update(
    mut measurements: ScaleMeasurementUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    measurements.handle(&state.db).await?;
    Ok(warp::reply::html("finished"))
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

pub async fn user(user: LoggedUser) -> WarpResult<impl Reply> {
    Ok(warp::reply::json(&user))
}

pub async fn add_garmin_correction(
    payload: AddGarminCorrectionRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    payload.handle(&state.db, &state.config).await?;
    Ok(warp::reply::html("finised"))
}

pub async fn fitbit_activity_types(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let result = FitbitActivityTypesRequest {}.handle(&state.config).await?;
    Ok(warp::reply::json(&result))
}

pub async fn strava_athlete(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let result = StravaAthleteRequest {}.handle(&state.config).await?;
    Ok(warp::reply::json(&result))
}

pub async fn fitbit_profile(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let result = FitbitProfileRequest {}.handle(&state.config).await?;
    Ok(warp::reply::json(&result))
}

pub async fn garmin_connect_activities(
    query: GarminConnectActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let result = query.handle(&state.connect_proxy).await?;
    Ok(warp::reply::json(&result))
}

pub async fn garmin_connect_activities_db(
    query: StravaActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let alist = GarminConnectActivitiesDBRequest(query)
        .handle(&state.db)
        .await?;
    Ok(warp::reply::json(&alist))
}

pub async fn garmin_connect_activities_db_update(
    payload: GarminConnectActivitiesDBUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body = payload.handle(&state.db).await?.join("\n");
    Ok(warp::reply::html(body))
}

pub async fn garmin_connect_user_summary(
    query: GarminConnectUserSummaryRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let js = query.handle(&state.connect_proxy).await?;
    Ok(warp::reply::json(&js))
}

pub async fn fitbit_activities_db(
    query: StravaActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let alist = FitbitActivitiesDBRequest(query).handle(&state.db).await?;
    Ok(warp::reply::json(&alist))
}

pub async fn fitbit_activities_db_update(
    payload: FitbitActivitiesDBUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body = payload.handle(&state.db).await?.join("\n");
    Ok(warp::reply::html(body))
}

pub async fn heartrate_statistics_summary_db(
    query: StravaActivitiesRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let alist = HeartrateStatisticsSummaryDBRequest(query)
        .handle(&state.db)
        .await?;
    Ok(warp::reply::json(&alist))
}

pub async fn heartrate_statistics_summary_db_update(
    payload: HeartrateStatisticsSummaryDBUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let body = payload.handle(&state.db).await?.join("\n");
    Ok(warp::reply::html(body))
}

async fn race_result_plot_impl(
    req: RaceResultPlotRequest,
    state: AppState,
    session: Session,
) -> Result<StackString, Error> {
    let is_demo = req.demo.unwrap_or(true);
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = req.handle(&state.db).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    Ok(HBR.render("GARMIN_TEMPLATE", &params)?.into())
}

pub async fn race_result_plot(
    mut query: RaceResultPlotRequest,
    _: LoggedUser,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    query.demo = Some(false);
    let session = session.unwrap_or_default();
    let body: String = race_result_plot_impl(query, state, session).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn race_result_plot_demo(
    mut query: RaceResultPlotRequest,
    state: AppState,
    session: Option<Session>,
) -> WarpResult<impl Reply> {
    query.demo = Some(true);
    let session = session.unwrap_or_default();
    let body: String = race_result_plot_impl(query, state, session).await?.into();
    Ok(warp::reply::html(body))
}

pub async fn race_result_flag(
    query: RaceResultFlagRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let result: String = query.handle(&state.db).await?.into();
    Ok(warp::reply::html(result))
}

pub async fn race_result_import(
    query: RaceResultImportRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    query.handle(&state.db).await?;
    Ok(warp::reply::html(""))
}

pub async fn race_results_db(
    query: RaceResultsDBRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let results = query.handle(&state.db).await?;
    Ok(warp::reply::json(&results))
}

pub async fn race_results_db_update(
    payload: RaceResultsDBUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    payload.handle(&state.db).await?;
    Ok(warp::reply::html(""))
}
