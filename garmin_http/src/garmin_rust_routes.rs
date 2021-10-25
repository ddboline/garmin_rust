#![allow(clippy::needless_pass_by_value)]

use itertools::Itertools;
use log::info;
use rweb::{
    get,
    multipart::{FormData, Part},
    post, Buf, Filter, Json, Query, Rejection, Schema,
};
use rweb_helper::{
    html_response::HtmlResponse as HtmlBase, json_response::JsonResponse as JsonBase, RwebResponse,
};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{collections::HashMap, convert::Infallible, string::ToString};
use tempdir::TempDir;
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_stream::StreamExt;

use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{
    common::{
        garmin_config::GarminConfig,
        garmin_templates::{get_buttons, get_scripts, get_style, HBR},
    },
    utils::{garmin_util::METERS_PER_MILE, iso_8601_datetime::convert_datetime_to_str},
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
    logged_user::{LoggedUser, Session},
    FitbitActivityWrapper, FitbitBodyWeightFatUpdateOutputWrapper, FitbitBodyWeightFatWrapper,
    FitbitHeartRateWrapper, FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper,
    GarminConnectUserDailySummaryWrapper, RaceResultsWrapper, ScaleMeasurementWrapper,
    StravaActivityWrapper,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

#[derive(Deserialize, Schema)]
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

    let req = GarminCli::process_pattern(config, filter_iter);
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

fn optional_session() -> impl Filter<Extract = (Option<Session>,), Error = Infallible> + Copy {
    rweb::cookie::optional("session")
}

#[derive(RwebResponse)]
#[response(description = "Main Page", content = "html")]
struct IndexResponse(HtmlBase<String, Error>);

#[get("/garmin/index.html")]
pub async fn garmin(
    query: Query<FilterRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<IndexResponse> {
    let query = query.into_inner();

    let mut session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;

    let body = garmin_body(query, &state, &mut session.history, false).await?;

    user.set_session(&state.client, &state.config, &session)
        .await
        .map_err(Into::<Error>::into)?;

    Ok(HtmlBase::new(body).into())
}

async fn garmin_body(
    query: FilterRequest,
    state: &AppState,
    history: &mut Vec<StackString>,
    is_demo: bool,
) -> HttpResult<String> {
    let grec = proc_pattern_wrapper(&state.config, query, history, is_demo);
    if history.len() > 5 {
        history.remove(0);
    }
    history.push(grec.request.filter.as_str().into());
    let body = grec.handle(&state.db).await?;

    Ok(body.into())
}

#[get("/garmin/demo.html")]
pub async fn garmin_demo(
    query: Query<FilterRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<IndexResponse> {
    let mut session = session.unwrap_or_default();
    let body = garmin_body(query.into_inner(), &state, &mut session.history, true).await?;
    let jwt = session.get_jwt_cookie(&state.config.domain);
    Ok(HtmlBase::new(body)
        .with_cookie(&jwt.encoded().to_string())
        .into())
}

#[derive(RwebResponse)]
#[response(description = "Upload Response", content = "html", status = "CREATED")]
struct UploadResponse(HtmlBase<String, Error>);

#[post("/garmin/upload_file")]
pub async fn garmin_upload(
    #[filter = "rweb::multipart::form"] form: FormData,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UploadResponse> {
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = garmin_upload_body(form, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

async fn garmin_upload_body(
    mut form: FormData,
    state: AppState,
    session: Session,
) -> HttpResult<String> {
    let tempdir = TempDir::new("garmin")?;
    let tempdir_str = tempdir.path().to_string_lossy().to_string();
    let mut fname = String::new();

    while let Some(item) = form.next().await {
        let item = item?;
        let filename = item.filename().unwrap_or("");
        if filename.is_empty() {
            return Err(Error::BadRequest("Empty Filename".into()));
        }
        fname = format!("{}/{}", tempdir_str, filename,);
        let file_size = save_file(&fname, item).await?;
        if file_size == 0 {
            return Err(Error::BadRequest("Empty File".into()));
        }
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

async fn save_file(file_path: &str, field: Part) -> Result<u64, anyhow::Error> {
    let mut file = File::create(file_path).await?;
    let mut stream = field.stream();

    while let Some(chunk) = stream.next().await {
        file.write_all(chunk?.chunk()).await?;
    }
    let file_size = file.metadata().await?.len();
    Ok(file_size)
}

#[derive(RwebResponse)]
#[response(description = "Connect Sync")]
struct ConnectSyncResponse(JsonBase<Vec<String>, Error>);

#[get("/garmin/garmin_connect_sync")]
pub async fn garmin_connect_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ConnectSyncResponse> {
    let body = GarminConnectSyncRequest {}
        .handle(&state.db, &state.connect_proxy)
        .await?
        .into_iter()
        .map(|x| x.to_string_lossy().into())
        .collect();
    Ok(JsonBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Connect Sync", content = "html")]
struct ConnectHrSyncResponse(HtmlBase<String, Error>);

#[get("/garmin/garmin_connect_hr_sync")]
pub async fn garmin_connect_hr_sync(
    query: Query<GarminConnectHrSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ConnectHrSyncResponse> {
    let heartrates = query
        .into_inner()
        .handle(&state.db, &state.connect_proxy, &state.config)
        .await?;
    let body: String = heartrates.to_table(Some(20)).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Connect Heartrate")]
struct ConnectHrApiResponse(JsonBase<Vec<FitbitHeartRateWrapper>, Error>);

#[get("/garmin/garmin_connect_hr_api")]
pub async fn garmin_connect_hr_api(
    query: Query<GarminConnectHrApiRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ConnectHrApiResponse> {
    let hr_vals = query
        .into_inner()
        .handle(state.connect_proxy)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hr_vals).into())
}

#[derive(RwebResponse)]
#[response(description = "Garmin Sync", content = "html")]
struct GarminSyncResponse(HtmlBase<String, Error>);

#[get("/garmin/garmin_sync")]
pub async fn garmin_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminSyncResponse> {
    let body = GarminSyncRequest {}.handle(&state.db).await?;
    let body = format!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Sync", content = "html")]
struct StravaSyncResponse(HtmlBase<String, Error>);

#[get("/garmin/strava_sync")]
pub async fn strava_sync(
    query: Query<StravaSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaSyncResponse> {
    let body = query
        .into_inner()
        .handle(&state.db, &state.config)
        .await?
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .join("\n");
    let body = format!(r#"<textarea cols=100 rows=40>{}</textarea>"#, body);
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Auth", content = "html")]
struct StravaAuthResponse(HtmlBase<String, Error>);

#[get("/garmin/strava/auth")]
pub async fn strava_auth(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaAuthResponse> {
    let body: String = StravaAuthRequest {}.handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Refresh Auth", content = "html")]
struct StravaRefreshResponse(HtmlBase<String, Error>);

#[get("/garmin/strava/refresh_auth")]
pub async fn strava_refresh(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaRefreshResponse> {
    let body: String = StravaRefreshRequest {}.handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Callback", content = "html")]
struct StravaCallbackResponse(HtmlBase<String, Error>);

#[get("/garmin/strava/callback")]
pub async fn strava_callback(
    query: Query<StravaCallbackRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaCallbackResponse> {
    let body: String = query.into_inner().handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Activities")]
struct StravaActivitiesResponse(JsonBase<Vec<StravaActivityWrapper>, Error>);

#[get("/garmin/strava/activities")]
pub async fn strava_activities(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesResponse> {
    let alist = query
        .into_inner()
        .handle(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(alist).into())
}

#[get("/garmin/strava/activities_db")]
pub async fn strava_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesResponse> {
    let alist = StravaActivitiesDBRequest(query.into_inner())
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(alist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Strava Activities Update",
    status = "CREATED",
    content = "html"
)]
struct StravaActivitiesUpdateResponse(HtmlBase<String, Error>);

#[post("/garmin/strava/activities_db")]
pub async fn strava_activities_db_update(
    payload: Json<StravaActiviesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesUpdateResponse> {
    let body = payload.into_inner().handle(&state.db).await?;
    let body = body.join("\n");
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Upload", status = "CREATED", content = "html")]
struct StravaUploadResponse(HtmlBase<String, Error>);

#[post("/garmin/strava/upload")]
pub async fn strava_upload(
    payload: Json<StravaUploadRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaUploadResponse> {
    let body: String = payload.into_inner().handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Update", status = "CREATED", content = "html")]
struct StravaUpdateResponse(HtmlBase<String, Error>);

#[post("/garmin/strava/update")]
pub async fn strava_update(
    payload: Json<StravaUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaUpdateResponse> {
    let body: String = payload.into_inner().handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Create", status = "CREATED", content = "html")]
struct StravaCreateResponse(HtmlBase<String, Error>);

#[post("/garmin/strava/create")]
pub async fn strava_create(
    query: Query<StravaCreateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaCreateResponse> {
    let activity_id = query.into_inner().handle(&state.db, &state.config).await?;
    let body = activity_id.map_or_else(|| "".into(), |activity_id| activity_id.to_string());
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Auth", content = "html")]
struct FitbitAuthResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/auth")]
pub async fn fitbit_auth(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitAuthResponse> {
    let body: String = FitbitAuthRequest {}.handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Refresh Auth", content = "html")]
struct FitbitRefreshResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/refresh_auth")]
pub async fn fitbit_refresh(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitRefreshResponse> {
    let body: String = FitbitRefreshRequest {}.handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate")]
struct FitbitHeartRateResponse(JsonBase<Vec<FitbitHeartRateWrapper>, Error>);

#[get("/garmin/fitbit/heartrate_api")]
pub async fn fitbit_heartrate_api(
    query: Query<FitbitHeartrateApiRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartRateResponse> {
    let hlist = query
        .into_inner()
        .handle(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hlist).into())
}

#[get("/garmin/fitbit/heartrate_cache")]
pub async fn fitbit_heartrate_cache(
    query: Query<FitbitHeartrateCacheRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartRateResponse> {
    let hlist = query
        .into_inner()
        .handle(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hlist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Fitbit Heartrate Update",
    content = "html",
    status = "CREATED"
)]
struct FitbitHeartrateUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/garmin/fitbit/heartrate_cache")]
pub async fn fitbit_heartrate_cache_update(
    payload: Json<FitbitHeartrateUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartrateUpdateResponse> {
    payload.into_inner().handle(&state.config).await?;
    Ok(HtmlBase::new("Finished").into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Body Weight")]
struct FitbitBodyWeightFatResponse(JsonBase<Vec<FitbitBodyWeightFatWrapper>, Error>);

#[get("/garmin/fitbit/bodyweight")]
pub async fn fitbit_bodyweight(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitBodyWeightFatResponse> {
    let hlist = FitbitBodyWeightFatRequest {}
        .handle(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hlist).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Body Weight Sync")]
struct FitbitBodyWeightFatUpdateResponse(JsonBase<FitbitBodyWeightFatUpdateOutputWrapper, Error>);

#[get("/garmin/fitbit/bodyweight_sync")]
pub async fn fitbit_bodyweight_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitBodyWeightFatUpdateResponse> {
    let hlist = FitbitBodyWeightFatUpdateRequest {}
        .handle(&state.db, &state.config)
        .await?;
    Ok(JsonBase::new(hlist.into()).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activities")]
struct FitbitActivitiesResponse(JsonBase<Vec<FitbitActivityWrapper>, Error>);

#[get("/garmin/fitbit/fitbit_activities")]
pub async fn fitbit_activities(
    query: Query<FitbitActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivitiesResponse> {
    let hlist = query
        .into_inner()
        .handle(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hlist).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Callback", content = "html")]
struct FitbitCallbackResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/callback")]
pub async fn fitbit_callback(
    query: Query<FitbitCallbackRequest>,
    #[data] state: AppState,
) -> WarpResult<FitbitCallbackResponse> {
    let body: String = query.into_inner().handle(&state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Sync", content = "html")]
struct FitbitSyncResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/sync")]
pub async fn fitbit_sync(
    query: Query<FitbitSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitSyncResponse> {
    let heartrates = query.into_inner().handle(&state.db, &state.config).await?;
    let start = if heartrates.len() > 20 {
        heartrates.len() - 20
    } else {
        0
    };
    let body: String = FitbitHeartRate::create_table(&heartrates[start..]).into();
    Ok(HtmlBase::new(body).into())
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

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate Statistics Plots", content = "html")]
struct FitbitStatisticsPlotResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/heartrate_statistics_plots")]
pub async fn heartrate_statistics_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let query: FitbitStatisticsPlotRequest = query.into_inner().into();
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = heartrate_statistics_plots_impl(query, state, session)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[get("/garmin/fitbit/heartrate_statistics_plots_demo")]
pub async fn heartrate_statistics_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let mut query: FitbitStatisticsPlotRequest = query.into_inner().into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let body: String = heartrate_statistics_plots_impl(query, state, session)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
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

#[derive(RwebResponse)]
#[response(description = "Scale Measurement Plots", content = "html")]
struct ScaleMeasurementResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/plots")]
pub async fn fitbit_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();
    let body = fitbit_plots_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[get("/garmin/fitbit/plots_demo")]
pub async fn fitbit_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = session.unwrap_or_default();
    let mut query: ScaleMeasurementPlotRequest = query.into_inner().into();
    query.is_demo = true;
    let body = fitbit_plots_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

async fn heartrate_plots_impl(
    query: FitbitHeartratePlotRequest,
    state: AppState,
    session: Session,
) -> HttpResult<String> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    info!("buttons {}", buttons);
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

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate Plots", content = "html")]
struct FitbitHeartratePlotResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/heartrate_plots")]
pub async fn heartrate_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let query: FitbitHeartratePlotRequest = query.into_inner().into();
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = heartrate_plots_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[get("/garmin/fitbit/heartrate_plots_demo")]
pub async fn heartrate_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let mut query: FitbitHeartratePlotRequest = query.into_inner().into();
    query.is_demo = true;
    let session = session.unwrap_or_default();
    let body = heartrate_plots_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Tcx Sync")]
struct FitbitTcxSyncResponse(JsonBase<Vec<String>, Error>);

#[get("/garmin/fitbit/fitbit_tcx_sync")]
pub async fn fitbit_tcx_sync(
    query: Query<FitbitTcxSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitTcxSyncResponse> {
    let flist = query
        .into_inner()
        .handle(&state.db, &state.config)
        .await?
        .into_iter()
        .map(|x| x.to_string_lossy().into())
        .collect();
    Ok(JsonBase::new(flist).into())
}

#[derive(RwebResponse)]
#[response(description = "Scale Measurements")]
struct ScaleMeasurementsResponse(JsonBase<Vec<ScaleMeasurementWrapper>, Error>);

#[get("/garmin/scale_measurements")]
pub async fn scale_measurement(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementsResponse> {
    let slist = query
        .into_inner()
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(slist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Scale Measurements Update",
    content = "html",
    status = "CREATED"
)]
struct ScaleMeasurementsUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/garmin/scale_measurements")]
pub async fn scale_measurement_update(
    measurements: Json<ScaleMeasurementUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementsUpdateResponse> {
    measurements.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Finished").into())
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

#[derive(RwebResponse)]
#[response(description = "Logged in User")]
struct UserResponse(JsonBase<LoggedUser, Error>);

#[allow(clippy::unused_async)]
#[get("/garmin/user")]
pub async fn user(#[filter = "LoggedUser::filter"] user: LoggedUser) -> WarpResult<UserResponse> {
    Ok(JsonBase::new(user).into())
}

#[derive(RwebResponse)]
#[response(description = "Logged in User", content = "html", status = "CREATED")]
struct AddGarminCorrectionResponse(HtmlBase<&'static str, Error>);

#[post("/garmin/add_garmin_correction")]
pub async fn add_garmin_correction(
    payload: Json<AddGarminCorrectionRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<AddGarminCorrectionResponse> {
    payload.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("finised").into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activity Types")]
struct FitbitActivityTypesResponse(JsonBase<HashMap<String, StackString>, Error>);

#[get("/garmin/fitbit/fitbit_activity_types")]
pub async fn fitbit_activity_types(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivityTypesResponse> {
    let result = FitbitActivityTypesRequest {}.handle(&state.config).await?;
    Ok(JsonBase::new(result).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Athlete")]
struct StravaAthleteResponse(HtmlBase<String, Error>);

#[get("/garmin/strava/athlete")]
pub async fn strava_athlete(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaAthleteResponse> {
    let result = StravaAthleteRequest {}.handle(&state.config).await?;
    let clubs = if let Some(clubs) = &result.clubs {
        let lines = clubs
            .iter()
            .map(|c| {
                format!(
                    r#"
                    <tr>
                    <td>{id}</td>
                    <td>{name}</td>
                    <td>{sport_type}</td>
                    <td>{city}</td>
                    <td>{state}</td>
                    <td>{country}</td>
                    <td>{private}</td>
                    <td>{member_count}</td>
                    <td>{url}</td>
                    </tr>
                "#,
                    id = c.id,
                    name = c.name,
                    sport_type = c.sport_type,
                    city = c.city,
                    state = c.state,
                    country = c.country,
                    private = c.private,
                    member_count = c.member_count,
                    url = c.url,
                )
            })
            .join("\n");
        format!(
            r#"
                <br>Clubs<br>
                <table border=1>
                <thead>
                <th>ID</th>
                <th>Name</th>
                <th>Sport Type</th>
                <th>City</th>
                <th>State</th>
                <th>Country</th>
                <th>Private</th>
                <th>Member Count</th>
                <th>Url</th>
                </thead>
                <tbody>
                {}
                </tbody>
                </table>
            "#,
            lines
        )
    } else {
        "".into()
    };
    let shoes = if let Some(shoes) = &result.shoes {
        let lines = shoes
            .iter()
            .map(|s| {
                format!(
                    r#"
                    <tr>
                    <td>{id}</td>
                    <td>{resource_state}</td>
                    <td>{primary}</td>
                    <td>{name}</td>
                    <td>{distance:0.2}</td>
                "#,
                    id = s.id,
                    resource_state = s.resource_state,
                    primary = s.primary,
                    name = s.name,
                    distance = s.distance / METERS_PER_MILE,
                )
            })
            .join("\n");
        format!(
            r#"
                <br>Shoes<br>
                <table border=1>
                <thead>
                <th>ID</th>
                <th>Resource State</th>
                <th>Primary</th>
                <th>Name</th>
                <th>Distance (mi)</th>
                </thead>
                <tbody>{}</tbody>
                </table>
            "#,
            lines
        )
    } else {
        "".into()
    };
    let body = format!(
        r#"
            <table border=1>
            <tbody>
            <tr><td>ID</td><td>{id}</td></tr>
            <tr><td>Username</td><td>{username}</td></tr>
            <tr><td>First Name</td><td>{firstname}</td></tr>
            <tr><td>Last Name</td><td>{lastname}</td></tr>
            <tr><td>City</td><td>{city}</td></tr>
            <tr><td>State</td><td>{state}</td></tr>
            <tr><td>Sex</td><td>{sex}</td></tr>
            <tr><td>Weight</td><td>{weight}</td></tr>
            <tr><td>Created At</td><td>{created_at}</td></tr>
            <tr><td>Updated At</td><td>{updated_at}</td></tr>
            {follower_count}{friend_count}{measurement_preference}
            </tbody>
            </table>
            {clubs}{shoes}
        "#,
        id = result.id,
        username = result.username,
        firstname = result.firstname,
        lastname = result.lastname,
        city = result.city,
        state = result.state,
        sex = result.sex,
        weight = result.weight,
        created_at = result.created_at,
        updated_at = result.updated_at,
        follower_count = if let Some(follower_count) = result.follower_count {
            format!(
                "<tr><td>Follower Count</td><td>{}</td></tr>",
                follower_count
            )
        } else {
            String::new()
        },
        friend_count = if let Some(friend_count) = result.friend_count {
            format!("<tr><td>Friend Count</td><td>{}</td></tr>", friend_count)
        } else {
            String::new()
        },
        measurement_preference = if let Some(measurement_preference) = result.measurement_preference
        {
            format!(
                "<tr><td>Measurement Preference</td><td>{}</td></tr>",
                measurement_preference
            )
        } else {
            String::new()
        },
        clubs = clubs,
        shoes = shoes,
    );
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Profile")]
struct FitbitProfileResponse(HtmlBase<String, Error>);

#[get("/garmin/fitbit/profile")]
pub async fn fitbit_profile(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitProfileResponse> {
    let result = FitbitProfileRequest {}.handle(&state.config).await?;
    let body = format!(
        r#"
            <table border=1>
            <tbody>
            <tr><td>Encoded ID</td><td>{encoded_id}</td></tr>
            <tr><td>First Name</td><td>{first_name}</td></tr>
            <tr><td>Last Name</td><td>{last_name}</td></tr>
            <tr><td>Full Name</td><td>{full_name}</td></tr>
            <tr><td>Avg Daily Steps</td><td>{average_daily_steps}</td></tr>
            <tr><td>Country</td><td>{country}</td></tr>
            <tr><td>DOB</td><td>{date_of_birth}</td></tr>
            <tr><td>Display Name</td><td>{display_name}</td></tr>
            <tr><td>Distance Unit</td><td>{distance_unit}</td></tr>
            <tr><td>Gender</td><td>{gender}</td></tr>
            <tr><td>Height</td><td>{height:0.2}</td></tr>
            <tr><td>Height Unit</td><td>{height_unit}</td></tr>
            <tr><td>Timezone</td><td>{timezone}</td></tr>
            <tr><td>Offset</td><td>{offset_from_utc_millis}</td></tr>
            <tr><td>Stride Length Running</td><td>{stride_length_running:0.2}</td></tr>
            <tr><td>Stride Length Walking</td><td>{stride_length_walking:0.2}</td></tr>
            <tr><td>Weight</td><td>{weight}</td></tr>
            <tr><td>Weight Unit</td><td>{weight_unit}</td></tr>
            </tbody>
            </table>
        "#,
        average_daily_steps = result.average_daily_steps,
        country = result.country,
        date_of_birth = result.date_of_birth,
        display_name = result.display_name,
        distance_unit = result.distance_unit,
        encoded_id = result.encoded_id,
        first_name = result.first_name,
        last_name = result.last_name,
        full_name = result.full_name,
        gender = result.gender,
        height = result.height,
        height_unit = result.height_unit,
        timezone = result.timezone,
        offset_from_utc_millis = result.offset_from_utc_millis,
        stride_length_running = result.stride_length_running,
        stride_length_walking = result.stride_length_walking,
        weight = result.weight,
        weight_unit = result.weight_unit,
    );
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Garmin Connect Activities")]
struct GarminConnectActivitiesResponse(JsonBase<Vec<GarminConnectActivityWrapper>, Error>);

#[get("/garmin/garmin_connect_activities")]
pub async fn garmin_connect_activities(
    query: Query<GarminConnectActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesResponse> {
    let result = query
        .into_inner()
        .handle(&state.connect_proxy)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(result).into())
}

#[get("/garmin/garmin_connect_activities_db")]
pub async fn garmin_connect_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesResponse> {
    let alist = GarminConnectActivitiesDBRequest(query.into_inner())
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(alist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Garmin Connect Activities",
    content = "html",
    status = "CREATED"
)]
struct GarminConnectActivitiesUpdateResponse(HtmlBase<String, Error>);

#[post("/garmin/garmin_connect_activities_db")]
pub async fn garmin_connect_activities_db_update(
    payload: Json<GarminConnectActivitiesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesUpdateResponse> {
    let body = payload.into_inner().handle(&state.db).await?.join("\n");
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Garmin Connect User Summary")]
struct GarminConnectUserSummaryResponse(JsonBase<GarminConnectUserDailySummaryWrapper, Error>);

#[get("/garmin/garmin_connect_user_summary")]
pub async fn garmin_connect_user_summary(
    query: Query<GarminConnectUserSummaryRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectUserSummaryResponse> {
    let js = query.into_inner().handle(&state.connect_proxy).await?;
    Ok(JsonBase::new(js.into()).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activities")]
struct FitbitActivitiesDBResponse(JsonBase<Vec<FitbitActivityWrapper>, Error>);

#[get("/garmin/fitbit/fitbit_activities_db")]
pub async fn fitbit_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivitiesDBResponse> {
    let alist = FitbitActivitiesDBRequest(query.into_inner())
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(alist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Fitbit Activities Update",
    content = "html",
    status = "CREATED"
)]
struct FitbitActivitiesDBUpdateResponse(HtmlBase<String, Error>);

#[post("/garmin/fitbit/fitbit_activities_db")]
pub async fn fitbit_activities_db_update(
    payload: Json<FitbitActivitiesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivitiesDBUpdateResponse> {
    let body = payload.into_inner().handle(&state.db).await?.join("\n");
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Heartrate Statistics")]
struct HeartrateStatisticsResponse(JsonBase<Vec<FitbitStatisticsSummaryWrapper>, Error>);

#[get("/garmin/fitbit/heartrate_statistics_summary_db")]
pub async fn heartrate_statistics_summary_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<HeartrateStatisticsResponse> {
    let alist = HeartrateStatisticsSummaryDBRequest(query.into_inner())
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(alist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Heartrate Statistics Update",
    content = "html",
    status = "CREATED"
)]
struct HeartrateStatisticsUpdateResponse(HtmlBase<String, Error>);

#[post("/garmin/fitbit/heartrate_statistics_summary_db")]
pub async fn heartrate_statistics_summary_db_update(
    payload: Json<HeartrateStatisticsSummaryDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<HeartrateStatisticsUpdateResponse> {
    let body = payload.into_inner().handle(&state.db).await?.join("\n");
    Ok(HtmlBase::new(body).into())
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

#[derive(RwebResponse)]
#[response(description = "Race Result Plot", content = "html")]
struct RaceResultPlotResponse(HtmlBase<String, Error>);

#[get("/garmin/race_result_plot")]
pub async fn race_result_plot(
    query: Query<RaceResultPlotRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultPlotResponse> {
    let mut query = query.into_inner();
    query.demo = Some(false);
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = race_result_plot_impl(query, state, session).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[get("/garmin/race_result_plot_demo")]
pub async fn race_result_plot_demo(
    query: Query<RaceResultPlotRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<RaceResultPlotResponse> {
    let mut query = query.into_inner();
    query.demo = Some(true);
    let session = session.unwrap_or_default();
    let body: String = race_result_plot_impl(query, state, session).await?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Race Result Plot", content = "html")]
struct RaceResultFlagResponse(HtmlBase<String, Error>);

#[get("/garmin/race_result_flag")]
pub async fn race_result_flag(
    query: Query<RaceResultFlagRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultFlagResponse> {
    let result: String = query.into_inner().handle(&state.db).await?.into();
    Ok(HtmlBase::new(result).into())
}

#[derive(RwebResponse)]
#[response(description = "Race Result Import", content = "html")]
struct RaceResultImportResponse(HtmlBase<&'static str, Error>);

#[get("/garmin/race_result_import")]
pub async fn race_result_import(
    query: Query<RaceResultImportRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultImportResponse> {
    query.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Finished").into())
}

#[derive(RwebResponse)]
#[response(description = "Race Results")]
struct RaceResultsResponse(JsonBase<Vec<RaceResultsWrapper>, Error>);

#[get("/garmin/race_results_db")]
pub async fn race_results_db(
    query: Query<RaceResultsDBRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultsResponse> {
    let results = query
        .into_inner()
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(results).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Race Results Update",
    status = "CREATED",
    content = "html"
)]
struct RaceResultsUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/garmin/race_results_db")]
pub async fn race_results_db_update(
    payload: Json<RaceResultsDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultsUpdateResponse> {
    payload.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Finished").into())
}
