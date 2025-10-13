#![allow(clippy::needless_pass_by_value)]
use axum::extract::{multipart::Field, DefaultBodyLimit, Json, Multipart, Query, State};
use derive_more::{From, Into};
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use log::debug;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::sync::Arc;
use tempfile::TempDir;
use time::Date;
use time_tz::OffsetDateTimeExt;
use tokio::{fs::File, io::AsyncWriteExt, task::spawn_blocking};
use tokio_stream::StreamExt;
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_helper::{
    html_response::HtmlResponse as HtmlBase, json_response::JsonResponse as JsonBase,
    UtoipaResponse,
};
use uuid::Uuid;

use fitbit_lib::{
    fitbit_archive, fitbit_heartrate::FitbitHeartRate,
    fitbit_statistics_summary::FitbitStatisticsSummary, scale_measurement::ScaleMeasurement,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::{
    date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
    errors::GarminError,
    garmin_config::GarminConfig,
};
use garmin_models::{
    fitbit_activity::FitbitActivity,
    garmin_connect_activity::GarminConnectActivity,
    garmin_correction_lap::GarminCorrectionLap,
    garmin_file,
    garmin_summary::{get_list_of_files_from_db, GarminSummary},
    strava_activity::StravaActivity,
};
use garmin_parser::garmin_parse::{GarminParse, GarminParseTrait};
use garmin_reports::garmin_summary_report_txt::create_report_query;
use garmin_utils::{garmin_util::titlecase, pgpool::PgPool};
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::StravaClient;
use utoipa::PartialSchema;

use crate::{
    errors::ServiceError as Error,
    garmin_elements::{
        garmin_connect_profile_body, index_new_body, scale_measurement_manual_input_body,
        strava_body, table_body, IndexConfig,
    },
    garmin_requests::{
        AddGarminCorrectionRequest, FitbitHeartrateCacheRequest, FitbitHeartratePlotRequest,
        FitbitHeartrateUpdateRequest, FitbitStatisticsPlotRequest,
        GarminConnectActivitiesDBUpdateRequest, GarminHtmlRequest,
        HeartrateStatisticsSummaryDBUpdateRequest, ScaleMeasurementPlotRequest,
        ScaleMeasurementRequest, ScaleMeasurementUpdateRequest, StravaActivitiesRequest,
        StravaCreateRequest, StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
    },
    garmin_rust_app::AppState,
    logged_user::{LoggedUser, Session},
    FitbitActivityWrapper, FitbitHeartRateWrapper, FitbitStatisticsSummaryWrapper,
    GarminConnectActivityWrapper, RaceResultsWrapper, RaceTypeWrapper, ScaleMeasurementWrapper,
    StravaActivityWrapper,
};

type WarpResult<T> = Result<T, Error>;

#[derive(Deserialize, ToSchema, IntoParams)]
struct FilterRequest {
    #[schema(inline)]
    #[param(inline)]
    filter: Option<StackString>,
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

    let filter_iter = filter.split(',');

    let req = GarminCli::process_pattern(config, filter_iter);
    let mut history: Vec<_> = history.iter().map(|s| s.as_ref().into()).collect();
    history.shrink_to_fit();

    GarminHtmlRequest {
        request: GarminRequest {
            filter: filter.into(),
            history,
            ..req
        },
        is_demo,
    }
}

#[derive(UtoipaResponse)]
#[response(description = "Main Page", content = "text/html")]
#[rustfmt::skip]
struct IndexResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/index.html",
    params(FilterRequest),
    responses(IndexResponse, Error)
)]
// Main Page
async fn garmin(
    state: State<Arc<AppState>>,
    query: Query<FilterRequest>,
    user: LoggedUser,
) -> WarpResult<IndexResponse> {
    let Query(query) = query;

    let mut session = user.get_session(&state.client, &state.config).await?;

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    if !session.history.contains(&grec.request.filter) {
        if session.history.len() > 5 {
            session.history.remove(0);
        }
        session.history.push(grec.request.filter.clone());
    }

    let body = get_index_body(&state.db, &state.config, &grec.request, false)
        .await?
        .into();

    user.set_session(&state.client, &state.config, &session)
        .await?;

    Ok(HtmlBase::new(body).into())
}

async fn get_index_body(
    pool: &PgPool,
    config: &GarminConfig,
    req: &GarminRequest,
    is_demo: bool,
) -> WarpResult<String> {
    let mut file_list: Vec<StackString> =
        get_list_of_files_from_db(&req.constraints.to_query_string(), pool)
            .await?
            .try_collect()
            .await?;
    file_list.shrink_to_fit();

    match file_list.len() {
        0 => Ok(String::new()),
        1 => {
            let file_name = file_list
                .first()
                .ok_or_else(|| GarminError::StaticCustomError("This shouldn't be happening..."))?;
            debug!("{}", &file_name);
            let avro_file = config.cache_dir.join(file_name.as_str());

            let gfile = if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                debug!("Cached avro file read: {}", avro_file.display());
                g
            } else {
                let gps_file = config.gps_dir.join(file_name.as_str());
                let mut corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;
                corr_map.shrink_to_fit();

                debug!("Reading gps_file: {}", gps_file.display());
                spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map)).await??
            };
            let sport: StackString = gfile.sport.into();
            let sport = titlecase(&sport);
            let dt = gfile.begin_datetime;
            let title = format_sstr!("Garmin Event {sport} at {dt}");
            let body = index_new_body(
                config,
                pool,
                title,
                is_demo,
                req.history.clone(),
                IndexConfig::File { gfile },
            )
            .await?;
            Ok(body)
        }
        _ => {
            let reports = create_report_query(pool, &req.options, &req.constraints).await?;
            let body = index_new_body(
                config,
                pool,
                "Garmin Summary".into(),
                is_demo,
                req.history.clone(),
                IndexConfig::Report { reports },
            )
            .await?;
            Ok(body)
        }
    }
}

#[utoipa::path(
    get,
    path = "/garmin/demo.html",
    params(FilterRequest),
    responses(IndexResponse, Error)
)]
// Demo Main Page
async fn garmin_demo(
    state: State<Arc<AppState>>,
    query: Query<FilterRequest>,
    session: Option<Session>,
) -> WarpResult<IndexResponse> {
    let Query(query) = query;

    let mut session = session.unwrap_or_default();

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    if !session.history.contains(&grec.request.filter) {
        if session.history.len() > 5 {
            session.history.remove(0);
        }
        session.history.push(grec.request.filter.clone());
    }

    let body = get_index_body(&state.db, &state.config, &grec.request, true)
        .await?
        .into();

    let jwt = session.get_jwt_cookie(&state.config.domain);
    let jwt_str = StackString::from_display(jwt.encoded());
    Ok(HtmlBase::new(body).with_cookie(&jwt_str).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Javascript", content = "text/javascript")]
#[rustfmt::skip]
struct JsResponse(HtmlBase::<&'static str>);

#[utoipa::path(get, path = "/garmin/scripts/garmin_scripts.js", responses(JsResponse))]
// Scripts
async fn garmin_scripts_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/garmin_scripts.js")).into()
}

#[utoipa::path(
    get,
    path = "/garmin/scripts/garmin_scripts_demo.js",
    responses(JsResponse)
)]
// Demo Scripts
async fn garmin_scripts_demo_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/garmin_scripts_demo.js")).into()
}

#[utoipa::path(get, path = "/garmin/scripts/line_plot.js", responses(JsResponse))]
async fn line_plot_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/line_plot.js")).into()
}

#[utoipa::path(get, path = "/garmin/scripts/scatter_plot.js", responses(JsResponse))]
async fn scatter_plot_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/scatter_plot.js")).into()
}

#[utoipa::path(
    get,
    path = "/garmin/scripts/scatter_plot_with_lines.js",
    responses(JsResponse)
)]
async fn scatter_plot_with_lines_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/scatter_plot_with_lines.js")).into()
}

#[utoipa::path(get, path = "/garmin/scripts/time_series.js", responses(JsResponse))]
async fn time_series_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/time_series.js")).into()
}

#[utoipa::path(get, path = "/garmin/scripts/initialize_map.js", responses(JsResponse))]
async fn initialize_map_js() -> JsResponse {
    HtmlBase::new(include_str!("../../templates/initialize_map.js")).into()
}

#[derive(UtoipaResponse)]
#[response(description = "Upload Response", content = "text/html", status = "CREATED")]
#[rustfmt::skip]
struct UploadResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/upload_file",
    request_body(content_type = "multipart/form-data"),
    responses(UploadResponse, Error)
)]
async fn garmin_upload(
    state: State<Arc<AppState>>,
    user: LoggedUser,
    form: Multipart,
) -> WarpResult<UploadResponse> {
    let session = user.get_session(&state.client, &state.config).await?;
    let body = garmin_upload_body(form, &state, session).await?;
    Ok(HtmlBase::new(body).into())
}

async fn garmin_upload_body(
    mut form: Multipart,
    state: &AppState,
    session: Session,
) -> WarpResult<StackString> {
    let tempdir = TempDir::with_prefix("garmin_rust")?;
    let tempdir_str = tempdir.path().to_string_lossy();
    let mut fname = StackString::new();

    while let Some(item) = form.next_field().await? {
        let filename = item.file_name().unwrap_or("");
        if filename.is_empty() {
            return Err(Error::BadRequest("Empty Filename".into()));
        }
        fname = format_sstr!("{tempdir_str}/{filename}");
        let file_size = save_file(fname.as_str(), item).await?;
        if file_size == 0 {
            return Err(Error::BadRequest("Empty File".into()));
        }
    }

    let filename = fname.as_str();
    let gcli = GarminCli::from_pool(&state.db)?;
    let filenames = vec![filename];
    let datetimes = gcli.process_filenames(&filenames).await?;
    gcli.sync_everything().await?;
    gcli.proc_everything().await?;

    let query = FilterRequest {
        filter: datetimes
            .first()
            .map(|dt| convert_datetime_to_str((*dt).into())),
    };

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    let body = get_index_body(&state.db, &state.config, &grec.request, false)
        .await?
        .into();
    Ok(body)
}

async fn save_file<'a>(file_path: &'a str, mut field: Field<'a>) -> Result<u64, Error> {
    let mut file = File::create(file_path).await?;
    let mut buf_size = 0usize;

    while let Some(chunk) = field.next().await {
        let chunk = chunk?;
        buf_size += chunk.len();
        file.write_all(&chunk).await?;
    }
    let file_size = file.metadata().await?.len();
    debug_assert!(buf_size as u64 == file_size);
    Ok(file_size)
}

#[derive(UtoipaResponse)]
#[response(description = "Garmin Sync", content = "text/html")]
#[rustfmt::skip]
struct GarminSyncResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/garmin_sync",
    responses(GarminSyncResponse, Error)
)]
async fn garmin_sync(state: State<Arc<AppState>>, _: LoggedUser) -> WarpResult<GarminSyncResponse> {
    let gcli = GarminCli::from_pool(&state.db).map_err(Into::<Error>::into)?;
    let mut body = gcli.sync_everything().await.map_err(Into::<Error>::into)?;
    body.extend_from_slice(&gcli.proc_everything().await.map_err(Into::<Error>::into)?);
    let body = body.join("\n").into();
    let body = table_body(body)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Sync", content = "text/html")]
#[rustfmt::skip]
struct StravaSyncResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/strava_sync",
    params(StravaSyncRequest),
    responses(StravaSyncResponse, Error)
)]
async fn strava_sync(
    state: State<Arc<AppState>>,
    query: Query<StravaSyncRequest>,
    _: LoggedUser,
) -> WarpResult<StravaSyncResponse> {
    let body = query
        .run_sync(&state.db, &state.config)
        .await?
        .into_iter()
        .map(|a| a.name)
        .join("\n")
        .into();
    let body = table_body(body)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Auth", content = "text/html")]
#[rustfmt::skip]
struct StravaAuthResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/strava/auth",
    responses(StravaAuthResponse, Error)
)]
async fn strava_auth(state: State<Arc<AppState>>, _: LoggedUser) -> WarpResult<StravaAuthResponse> {
    let client = StravaClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let body: StackString = client
        .get_authorization_url_api()
        .map_err(Into::<Error>::into)
        .map(|u| u.as_str().into())?;

    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Refresh Auth", content = "text/html")]
#[rustfmt::skip]
struct StravaRefreshResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/strava/refresh_auth",
    responses(StravaRefreshResponse, Error)
)]
async fn strava_refresh(
    state: State<Arc<AppState>>,
    _: LoggedUser,
) -> WarpResult<StravaRefreshResponse> {
    let mut client = StravaClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    client
        .refresh_access_token()
        .await
        .map_err(Into::<Error>::into)?;
    client.to_file().await.map_err(Into::<Error>::into)?;
    let body: StackString = r#"
        <title>Strava auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#
        .into();

    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema, IntoParams)]
// StravaCallbackRequest
struct StravaCallbackRequest {
    // Authorization Code
    #[schema(inline)]
    #[param(inline)]
    code: StackString,
    // CSRF State
    #[schema(inline)]
    #[param(inline)]
    state: StackString,
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Callback", content = "text/html")]
#[rustfmt::skip]
struct StravaCallbackResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/strava/callback",
    params(StravaCallbackRequest),
    responses(StravaCallbackResponse, Error)
)]
async fn strava_callback(
    state: State<Arc<AppState>>,
    query: Query<StravaCallbackRequest>,
    _: LoggedUser,
) -> WarpResult<StravaCallbackResponse> {
    let Query(query) = query;
    let mut client = StravaClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    client
        .process_callback(&query.code, &query.state)
        .await
        .map_err(Into::<Error>::into)?;
    client.to_file().await.map_err(Into::<Error>::into)?;
    let body: StackString = r#"
        <title>Strava auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(ToSchema, Serialize, Into, From)]
struct StravaActivityList(Vec<StravaActivityWrapper>);

#[derive(UtoipaResponse)]
#[response(description = "Strava Activities")]
#[rustfmt::skip]
struct StravaActivitiesResponse(JsonBase::<StravaActivityList>);

#[utoipa::path(
    get,
    path = "/garmin/strava/activities",
    params(StravaActivitiesRequest),
    responses(StravaActivitiesResponse, Error)
)]
async fn strava_activities(
    state: State<Arc<AppState>>,
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
) -> WarpResult<StravaActivitiesResponse> {
    let mut alist: Vec<_> = query
        .get_activities(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    alist.shrink_to_fit();
    Ok(JsonBase::new(alist.into()).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// Pagination
struct Pagination {
    // Total Number of Entries
    total: usize,
    // Number of Entries to Skip
    offset: usize,
    // Number of Entries Returned
    limit: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedStravaActivity
struct PaginatedStravaActivity {
    pagination: Pagination,
    data: Vec<StravaActivityWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Strava DB Activities")]
#[rustfmt::skip]
struct StravaActivitiesDBResponse(JsonBase::<PaginatedStravaActivity>);

#[utoipa::path(
    get,
    path = "/garmin/strava/activities_db",
    params(StravaActivitiesRequest),
    responses(StravaActivitiesDBResponse, Error)
)]
async fn strava_activities_db(
    state: State<Arc<AppState>>,
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
) -> WarpResult<StravaActivitiesDBResponse> {
    let Query(query) = query;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date;
    let end_date = query.end_date;

    let total = StravaActivity::get_total(&state.db, start_date, end_date)
        .await
        .map_err(Into::<Error>::into)?;
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let mut data: Vec<_> =
        StravaActivity::read_from_db(&state.db, start_date, end_date, Some(offset), Some(limit))
            .await
            .map_err(Into::<Error>::into)?
            .map_ok(Into::into)
            .try_collect()
            .await
            .map_err(Into::<Error>::into)?;
    data.shrink_to_fit();

    Ok(JsonBase::new(PaginatedStravaActivity { pagination, data }).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// StravaActiviesDBUpdateRequest
struct StravaActiviesDBUpdateRequest {
    updates: Vec<StravaActivityWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Strava Activities Update",
    status = "CREATED",
    content = "text/html"
)]
#[rustfmt::skip]
struct StravaActivitiesUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/strava/activities_db",
    request_body = StravaActiviesDBUpdateRequest,
    responses(StravaActivitiesUpdateResponse, Error)
)]
async fn strava_activities_db_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<StravaActiviesDBUpdateRequest>,
) -> WarpResult<StravaActivitiesUpdateResponse> {
    let Json(payload) = payload;
    let mut updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
    updates.shrink_to_fit();
    let body = StravaActivity::upsert_activities(&updates, &state.db)
        .await
        .map_err(Into::<Error>::into)?;
    StravaActivity::fix_summary_id_in_db(&state.db)
        .await
        .map_err(Into::<Error>::into)?;

    let body = body.join("\n").into();
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Upload", status = "CREATED", content = "text/html")]
#[rustfmt::skip]
struct StravaUploadResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/strava/upload",
    request_body = StravaUploadRequest,
    responses(StravaUploadResponse, Error)
)]
async fn strava_upload(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<StravaUploadRequest>,
) -> WarpResult<StravaUploadResponse> {
    let Json(payload) = payload;
    let body = payload.run_upload(&state.config).await?;
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Update", status = "CREATED", content = "text/html")]
#[rustfmt::skip]
struct StravaUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/strava/update",
    request_body = StravaUpdateRequest,
    responses(StravaUpdateResponse, Error)
)]
async fn strava_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<StravaUpdateRequest>,
) -> WarpResult<StravaUpdateResponse> {
    let Json(payload) = payload;
    let body = payload.run_update(&state.config).await?;
    Ok(HtmlBase::new(body.as_str().into()).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Create", status = "CREATED", content = "text/html")]
#[rustfmt::skip]
struct StravaCreateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/strava/create",
    params(StravaCreateRequest),
    responses(StravaCreateResponse, Error)
)]
async fn strava_create(
    state: State<Arc<AppState>>,
    query: Query<StravaCreateRequest>,
    _: LoggedUser,
) -> WarpResult<StravaCreateResponse> {
    let activity_id = query.create_activity(&state.db, &state.config).await?;
    let body = activity_id.map_or_else(|| "".into(), StackString::from_display);
    Ok(HtmlBase::new(body).into())
}

#[derive(ToSchema, Serialize, Into, From)]
struct FitbitHeartRateList(Vec<FitbitHeartRateWrapper>);

#[derive(UtoipaResponse)]
#[response(description = "Fitbit Heartrate")]
#[rustfmt::skip]
struct FitbitHeartRateResponse(JsonBase::<FitbitHeartRateList>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_cache",
    params(FitbitHeartrateCacheRequest),
    responses(FitbitHeartRateResponse, Error)
)]
async fn fitbit_heartrate_cache(
    state: State<Arc<AppState>>,
    query: Query<FitbitHeartrateCacheRequest>,
    _: LoggedUser,
) -> WarpResult<FitbitHeartRateResponse> {
    let Query(query) = query;
    let mut hlist: Vec<_> = query
        .get_cache(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    hlist.shrink_to_fit();
    Ok(JsonBase::new(hlist.into()).into())
}

#[derive(UtoipaResponse)]
#[response(
    description = "Fitbit Heartrate Update",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct FitbitHeartrateUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/fitbit/heartrate_cache",
    request_body = FitbitHeartrateUpdateRequest,
    responses(FitbitHeartrateUpdateResponse, Error)
)]
async fn fitbit_heartrate_cache_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<FitbitHeartrateUpdateRequest>,
) -> WarpResult<FitbitHeartrateUpdateResponse> {
    let Json(payload) = payload;
    let dates = payload.merge_data(&state.config).await?;
    Ok(HtmlBase::new(format_sstr!("Finished {dates:?}")).into())
}

#[derive(ToSchema, Serialize, Into, From)]
struct FitbitActivityList(Vec<FitbitActivityWrapper>);

#[derive(Serialize, Deserialize, ToSchema)]
struct FitbitSyncRequest {
    date: Date,
}

#[derive(UtoipaResponse)]
#[response(description = "Fitbit Heartrate Statistics Plots", content = "text/html")]
#[rustfmt::skip]
struct FitbitStatisticsPlotResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_statistics_plots",
    params(ScaleMeasurementRequest),
    responses(FitbitStatisticsPlotResponse, Error)
)]
async fn heartrate_statistics_plots(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    user: LoggedUser,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let Query(query) = query;
    let query: FitbitStatisticsPlotRequest = query.into();
    let session = user.get_session(&state.client, &state.config).await?;
    let mut stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        &state.db,
        Some(query.start_date),
        Some(query.end_date),
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    stats.shrink_to_fit();
    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        false,
        session.history,
        IndexConfig::HearRateSummary {
            stats,
            offset: Some(query.offset),
            start_date: Some(query.start_date),
            end_date: Some(query.end_date),
        },
    )
    .await?
    .into();
    Ok(HtmlBase::new(body).into())
}

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_statistics_plots_demo",
    params(ScaleMeasurementRequest),
    responses(FitbitStatisticsPlotResponse, Error)
)]
async fn heartrate_statistics_plots_demo(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    session: Option<Session>,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let Query(query) = query;
    let mut query: FitbitStatisticsPlotRequest = query.into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let mut stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        &state.db,
        Some(query.start_date),
        Some(query.end_date),
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    stats.shrink_to_fit();
    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        true,
        session.history,
        IndexConfig::HearRateSummary {
            stats,
            offset: Some(query.offset),
            start_date: Some(query.start_date),
            end_date: Some(query.end_date),
        },
    )
    .await?
    .into();

    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Scale Measurement Plots", content = "text/html")]
#[rustfmt::skip]
struct ScaleMeasurementResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/plots",
    params(ScaleMeasurementRequest),
    responses(ScaleMeasurementResponse, Error)
)]
async fn fitbit_plots(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    user: LoggedUser,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = user.get_session(&state.client, &state.config).await?;
    let Query(query) = query;
    let query: ScaleMeasurementPlotRequest = query.into();

    let measurements = ScaleMeasurement::read_from_db(
        &state.db,
        Some(query.start_date),
        Some(query.end_date),
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?;

    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        false,
        session.history,
        IndexConfig::Scale {
            measurements,
            offset: query.offset,
            start_date: query.start_date,
            end_date: query.end_date,
        },
    )
    .await?
    .into();

    Ok(HtmlBase::new(body).into())
}

#[utoipa::path(
    get,
    path = "/garmin/fitbit/plots_demo",
    params(ScaleMeasurementRequest),
    responses(ScaleMeasurementResponse, Error)
)]
async fn fitbit_plots_demo(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    session: Option<Session>,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = session.unwrap_or_default();
    let Query(query) = query;
    let mut query: ScaleMeasurementPlotRequest = query.into();
    query.is_demo = true;

    let measurements = ScaleMeasurement::read_from_db(
        &state.db,
        Some(query.start_date),
        Some(query.end_date),
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?;

    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        true,
        session.history,
        IndexConfig::Scale {
            measurements,
            offset: query.offset,
            start_date: query.start_date,
            end_date: query.end_date,
        },
    )
    .await?
    .into();

    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Fitbit Heartrate Plots", content = "text/html")]
#[rustfmt::skip]
struct FitbitHeartratePlotResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_plots",
    params(ScaleMeasurementRequest),
    responses(FitbitHeartratePlotResponse, Error)
)]
async fn heartrate_plots(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    user: LoggedUser,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let Query(query) = query;
    let query: FitbitHeartratePlotRequest = query.into();
    let session = user.get_session(&state.client, &state.config).await?;

    let parquet_values = fitbit_archive::get_number_of_heartrate_values(
        &state.config,
        query.start_date,
        query.end_date,
    )
    .map_err(Into::<Error>::into)?;
    let step_size = if parquet_values < 40_000 {
        1
    } else {
        parquet_values / 40_000
    };
    debug!("parquet_values {parquet_values} step size {step_size}");

    let heartrate = if parquet_values == 0 {
        FitbitHeartRate::get_heartrate_values(
            &state.config,
            &state.db,
            query.start_date,
            query.end_date,
        )
        .await
    } else {
        let config = state.config.clone();
        spawn_blocking(move || {
            fitbit_archive::get_heartrate_values(
                &config,
                query.start_date,
                query.end_date,
                Some(step_size),
            )
        })
        .await
        .map_err(Into::<Error>::into)?
    }
    .map_err(Into::<Error>::into)?;

    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        false,
        session.history,
        IndexConfig::HeartRate {
            heartrate,
            start_date: query.start_date,
            end_date: query.end_date,
            button_date: query.button_date,
        },
    )
    .await?
    .into();
    Ok(HtmlBase::new(body).into())
}

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_plots_demo",
    params(ScaleMeasurementRequest),
    responses(FitbitHeartratePlotResponse, Error)
)]
async fn heartrate_plots_demo(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    session: Option<Session>,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let Query(query) = query;
    let mut query: FitbitHeartratePlotRequest = query.into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let heartrate = FitbitHeartRate::get_heartrate_values(
        &state.config,
        &state.db,
        query.start_date,
        query.end_date,
    )
    .await
    .map_err(Into::<Error>::into)?;
    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        true,
        session.history,
        IndexConfig::HeartRate {
            heartrate,
            start_date: query.start_date,
            end_date: query.end_date,
            button_date: query.button_date,
        },
    )
    .await?
    .into();

    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedScaleMeasurement
struct PaginatedScaleMeasurement {
    pagination: Pagination,
    data: Vec<ScaleMeasurementWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Scale Measurements")]
#[rustfmt::skip]
struct ScaleMeasurementsResponse(JsonBase::<PaginatedScaleMeasurement>);

#[utoipa::path(
    get,
    path = "/garmin/scale_measurements",
    params(ScaleMeasurementRequest),
    responses(ScaleMeasurementsResponse, Error)
)]
async fn scale_measurement(
    state: State<Arc<AppState>>,
    query: Query<ScaleMeasurementRequest>,
    _: LoggedUser,
) -> WarpResult<ScaleMeasurementsResponse> {
    let Query(query) = query;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date;
    let end_date = query.end_date;

    let total = ScaleMeasurement::get_total(&state.db, start_date, end_date)
        .await
        .map_err(Into::<Error>::into)?;
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let mut data: Vec<_> = ScaleMeasurement::read_from_db(
        &state.db,
        query.start_date,
        query.end_date,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .into_iter()
    .map(Into::into)
    .collect();
    data.shrink_to_fit();

    Ok(JsonBase::new(PaginatedScaleMeasurement { pagination, data }).into())
}

#[derive(UtoipaResponse)]
#[response(
    description = "Scale Measurements Update",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct ScaleMeasurementsUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/garmin/scale_measurements",
    request_body = ScaleMeasurementUpdateRequest,
    responses(ScaleMeasurementsUpdateResponse, Error)
)]
async fn scale_measurement_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    measurements: Json<ScaleMeasurementUpdateRequest>,
) -> WarpResult<ScaleMeasurementsUpdateResponse> {
    let Json(measurements) = measurements;
    let mut measurements: Vec<_> = measurements
        .measurements
        .into_iter()
        .map(Into::into)
        .collect();
    measurements.shrink_to_fit();
    ScaleMeasurement::merge_updates(&mut measurements, &state.db)
        .await
        .map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new("Finished").into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// ScaleMeasurementManualRequest
struct ScaleMeasurementManualRequest {
    // Weight in lbs", example = r#""189.0""#)]
    weight_in_lbs: f64,
    // Body fat percent", example = r#""20.3""#)]
    body_fat_percent: f64,
    // Muscle mass in lbs", example = r#""153.0""#)]
    muscle_mass_lbs: f64,
    // Body water percent", example = r#""63.0""#)]
    body_water_percent: f64,
    // Bone mass in lbs", example = r#""63.0""#)]
    bone_mass_lbs: f64,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Scale Measurement Manual Input Post",
    status = "CREATED"
)]
#[rustfmt::skip]
struct ScaleMeasurementManualResponse(JsonBase::<ScaleMeasurementWrapper>);

#[utoipa::path(
    post,
    path = "/garmin/scale_measurements/manual",
    request_body = ScaleMeasurementManualRequest,
    responses(ScaleMeasurementManualResponse, Error)
)]
async fn scale_measurement_manual(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<ScaleMeasurementManualRequest>,
) -> WarpResult<ScaleMeasurementManualResponse> {
    let Json(payload) = payload;
    let mut measurement = ScaleMeasurement::from_fit_plus(
        payload.weight_in_lbs,
        payload.body_fat_percent,
        payload.muscle_mass_lbs,
        payload.body_water_percent,
        payload.bone_mass_lbs,
    )
    .map_err(Into::<Error>::into)?;
    measurement
        .insert_into_db(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let local = DateTimeWrapper::local_tz();
    let date = measurement.datetime.to_timezone(local).date();
    if let Ok(mut client) = GarminConnectClient::new(state.config.clone()) {
        if client.init().await.is_ok() && client.upload_weight(&measurement).await.is_ok() {
            if let Ok(weight) = client.get_weight(date).await {
                for dwl in &weight.date_weight_list {
                    if (dwl.weight - measurement.mass_in_grams()) < 1.0 {
                        let primary_key = dwl.sample_primary_key;
                        if measurement
                            .set_connect_primary_key(primary_key, &state.db)
                            .await
                            .is_ok()
                        {
                            debug!("set weight {weight:?}");
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(JsonBase::new(measurement.into()).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Scale Measurement Manual Input")]
#[rustfmt::skip]
struct ScaleMeasurementManualInputResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/scale_measurements/manual/input",
    responses(ScaleMeasurementManualInputResponse, Error)
)]
async fn scale_measurement_manual_input(
    _: LoggedUser,
) -> WarpResult<ScaleMeasurementManualInputResponse> {
    let body = scale_measurement_manual_input_body()?;
    Ok(HtmlBase::new(body.into()).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Logged in User")]
#[rustfmt::skip]
struct UserResponse(JsonBase::<LoggedUser>);

#[allow(clippy::unused_async)]
#[utoipa::path(get, path = "/garmin/user", responses(UserResponse))]
async fn user(user: LoggedUser) -> UserResponse {
    JsonBase::new(user).into()
}

#[derive(UtoipaResponse)]
#[response(description = "Add correction", content = "text/html", status = "CREATED")]
#[rustfmt::skip]
struct AddGarminCorrectionResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/garmin/add_garmin_correction",
    request_body = AddGarminCorrectionRequest,
    responses(AddGarminCorrectionResponse, Error)
)]
async fn add_garmin_correction(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<AddGarminCorrectionRequest>,
) -> WarpResult<AddGarminCorrectionResponse> {
    let Json(payload) = payload;
    payload.add_corrections(&state.db).await?;
    Ok(HtmlBase::new("finised").into())
}

#[derive(UtoipaResponse)]
#[response(description = "Strava Athlete")]
#[rustfmt::skip]
struct StravaAthleteResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/strava/athlete",
    responses(StravaAthleteResponse, Error)
)]
async fn strava_athlete(
    state: State<Arc<AppState>>,
    _: LoggedUser,
) -> WarpResult<StravaAthleteResponse> {
    let client = StravaClient::with_auth(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let result = client
        .get_strava_athlete()
        .await
        .map_err(Into::<Error>::into)?;
    let body = strava_body(result)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedGarminConnectActivity
struct PaginatedGarminConnectActivity {
    pagination: Pagination,
    data: Vec<GarminConnectActivityWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Garmin Connect Activities")]
#[rustfmt::skip]
struct GarminConnectActivitiesResponse(JsonBase::<PaginatedGarminConnectActivity>);

#[utoipa::path(
    get,
    path = "/garmin/garmin_connect_activities_db",
    params(StravaActivitiesRequest),
    responses(GarminConnectActivitiesResponse, Error)
)]
async fn garmin_connect_activities_db(
    state: State<Arc<AppState>>,
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
) -> WarpResult<GarminConnectActivitiesResponse> {
    let Query(query) = query;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date;
    let end_date = query.end_date;

    let total = GarminConnectActivity::get_total(&state.db, start_date, end_date)
        .await
        .map_err(Into::<Error>::into)?;

    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let mut data: Vec<_> = GarminConnectActivity::read_from_db(
        &state.db,
        start_date,
        end_date,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    data.shrink_to_fit();
    Ok(JsonBase::new(PaginatedGarminConnectActivity { pagination, data }).into())
}

#[derive(UtoipaResponse)]
#[response(
    description = "Garmin Connect Activities",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct GarminConnectActivitiesUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/garmin_connect_activities_db",
    request_body = GarminConnectActivitiesDBUpdateRequest,
    responses(GarminConnectActivitiesUpdateResponse, Error)
)]
async fn garmin_connect_activities_db_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<GarminConnectActivitiesDBUpdateRequest>,
) -> WarpResult<GarminConnectActivitiesUpdateResponse> {
    let Json(payload) = payload;
    let mut updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
    updates.shrink_to_fit();
    let body: StackString = GarminConnectActivity::upsert_activities(&updates, &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .join("\n")
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedFitbitActivity
struct PaginatedFitbitActivity {
    pagination: Pagination,
    data: Vec<FitbitActivityWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Fitbit Activities")]
#[rustfmt::skip]
struct FitbitActivitiesDBResponse(JsonBase::<PaginatedFitbitActivity>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/fitbit_activities_db",
    params(StravaActivitiesRequest),
    responses(FitbitActivitiesDBResponse, Error)
)]
async fn fitbit_activities_db(
    state: State<Arc<AppState>>,
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
) -> WarpResult<FitbitActivitiesDBResponse> {
    let Query(query) = query;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date;
    let end_date = query.end_date;

    let total = FitbitActivity::get_total(&state.db, start_date, end_date)
        .await
        .map_err(Into::<Error>::into)?;
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let mut data: Vec<_> =
        FitbitActivity::read_from_db(&state.db, start_date, end_date, Some(offset), Some(limit))
            .await
            .map_err(Into::<Error>::into)?
            .into_iter()
            .map(Into::into)
            .collect();
    data.shrink_to_fit();
    Ok(JsonBase::new(PaginatedFitbitActivity { pagination, data }).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct FitbitActivitiesDBUpdateRequest {
    updates: Vec<FitbitActivityWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Fitbit Activities Update",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct FitbitActivitiesDBUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/fitbit/fitbit_activities_db",
    request_body = FitbitActivitiesDBUpdateRequest,
    responses(FitbitActivitiesDBUpdateResponse, Error)
)]
async fn fitbit_activities_db_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<FitbitActivitiesDBUpdateRequest>,
) -> WarpResult<FitbitActivitiesDBUpdateResponse> {
    let Json(payload) = payload;
    let mut updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
    updates.shrink_to_fit();
    let body = FitbitActivity::upsert_activities(&updates, &state.db)
        .await
        .map_err(Into::<Error>::into)?;
    FitbitActivity::fix_summary_id_in_db(&state.db)
        .await
        .map_err(Into::<Error>::into)?;

    let body = body.join("\n").into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedFitbitStatisticsSummary
struct PaginatedFitbitStatisticsSummary {
    pagination: Pagination,
    data: Vec<FitbitStatisticsSummaryWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Heartrate Statistics")]
#[rustfmt::skip]
struct HeartrateStatisticsResponse(JsonBase::<PaginatedFitbitStatisticsSummary>);

#[utoipa::path(
    get,
    path = "/garmin/fitbit/heartrate_statistics_summary_db",
    params(StravaActivitiesRequest),
    responses(HeartrateStatisticsResponse, Error)
)]
async fn heartrate_statistics_summary_db(
    state: State<Arc<AppState>>,
    query: Query<StravaActivitiesRequest>,
    _: LoggedUser,
) -> WarpResult<HeartrateStatisticsResponse> {
    let Query(query) = query;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date;
    let end_date = query.end_date;

    let total = FitbitStatisticsSummary::get_total(&state.db, start_date, end_date)
        .await
        .map_err(Into::<Error>::into)?;

    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let mut data: Vec<_> = FitbitStatisticsSummary::read_from_db(
        &state.db,
        start_date,
        end_date,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(|x| {
        let x: FitbitStatisticsSummaryWrapper = x.into();
        x
    })
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    data.shrink_to_fit();
    Ok(JsonBase::new(PaginatedFitbitStatisticsSummary { pagination, data }).into())
}

#[derive(UtoipaResponse)]
#[response(
    description = "Heartrate Statistics Update",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct HeartrateStatisticsUpdateResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/garmin/fitbit/heartrate_statistics_summary_db",
    request_body = HeartrateStatisticsSummaryDBUpdateRequest,
    responses(HeartrateStatisticsUpdateResponse, Error)
)]
async fn heartrate_statistics_summary_db_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<HeartrateStatisticsSummaryDBUpdateRequest>,
) -> WarpResult<HeartrateStatisticsUpdateResponse> {
    let Json(payload) = payload;
    let body = payload.process_updates(&state.db).await?.join("\n").into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
// RaceResultPlotRequest
struct RaceResultPlotRequest {
    // Race Type
    race_type: RaceTypeWrapper,
    // Demo Flag
    demo: Option<bool>,
}

async fn race_result_plot_impl(
    req: RaceResultPlotRequest,
    state: &AppState,
    session: Session,
) -> Result<StackString, Error> {
    let model = RaceResultAnalysis::run_analysis(req.race_type.into(), &state.db).await?;
    let demo = req.demo.unwrap_or(true);

    let body = index_new_body(
        &state.config,
        &state.db,
        "".into(),
        demo,
        session.history,
        IndexConfig::RaceResult { model },
    )
    .await?
    .into();
    Ok(body)
}

#[derive(UtoipaResponse)]
#[response(description = "Race Result Plot", content = "text/html")]
#[rustfmt::skip]
struct RaceResultPlotResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/race_result_plot",
    params(RaceResultPlotRequest),
    responses(RaceResultPlotResponse, Error)
)]
async fn race_result_plot(
    state: State<Arc<AppState>>,
    query: Query<RaceResultPlotRequest>,
    user: LoggedUser,
) -> WarpResult<RaceResultPlotResponse> {
    let Query(mut query) = query;
    query.demo = Some(false);
    let session = user.get_session(&state.client, &state.config).await?;
    let body = race_result_plot_impl(query, &state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[utoipa::path(
    get,
    path = "/garmin/race_result_plot_demo",
    params(RaceResultPlotRequest),
    responses(RaceResultPlotResponse, Error)
)]
async fn race_result_plot_demo(
    query: Query<RaceResultPlotRequest>,
    state: State<Arc<AppState>>,
    session: Option<Session>,
) -> WarpResult<RaceResultPlotResponse> {
    let Query(mut query) = query;
    query.demo = Some(true);
    let session = session.unwrap_or_default();
    let body = race_result_plot_impl(query, &state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
struct RaceResultFlagRequest {
    id: Uuid,
}

#[derive(UtoipaResponse)]
#[response(description = "Race Result Plot", content = "text/html")]
#[rustfmt::skip]
struct RaceResultFlagResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/race_result_flag",
    params(RaceResultFlagRequest),
    responses(RaceResultFlagResponse, Error)
)]
async fn race_result_flag(
    query: Query<RaceResultFlagRequest>,
    _: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<RaceResultFlagResponse> {
    let Query(query) = query;

    let result = if let Some(mut result) = RaceResults::get_result_by_id(query.id, &state.db)
        .await
        .map_err(Into::<Error>::into)?
    {
        result.race_flag = !result.race_flag;
        let flag_str = StackString::from_display(result.race_flag);
        result
            .update_db(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
        flag_str
    } else {
        "".into()
    };
    Ok(HtmlBase::new(result).into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
struct RaceResultImportRequest {
    #[schema(inline)]
    #[param(inline)]
    filename: StackString,
}

#[derive(UtoipaResponse)]
#[response(description = "Race Result Import", content = "text/html")]
#[rustfmt::skip]
struct RaceResultImportResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    get,
    path = "/garmin/race_result_import",
    params(RaceResultImportRequest),
    responses(RaceResultImportResponse, Error)
)]
async fn race_result_import(
    query: Query<RaceResultImportRequest>,
    _: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<RaceResultImportResponse> {
    let Query(query) = query;

    if let Some(summary) = GarminSummary::get_by_filename(&state.db, query.filename.as_str())
        .await
        .map_err(Into::<Error>::into)?
    {
        let begin_datetime = summary.begin_datetime.into();
        let mut result: RaceResults = summary.into();
        if let Some(activity) = StravaActivity::get_by_begin_datetime(&state.db, begin_datetime)
            .await
            .map_err(Into::<Error>::into)?
        {
            result.race_name = Some(activity.name);
        }
        result
            .insert_into_db(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
        result
            .set_race_id(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
        result
            .update_race_summary_ids(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
    }
    Ok(HtmlBase::new("Finished").into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
struct RaceResultsDBRequest {
    // Race Type
    race_type: Option<RaceTypeWrapper>,
}

#[derive(ToSchema, Serialize, Into, From)]
struct RaceResultsList(Vec<RaceResultsWrapper>);

#[derive(UtoipaResponse)]
#[response(description = "Race Results")]
#[rustfmt::skip]
struct RaceResultsResponse(JsonBase::<RaceResultsList>);

#[utoipa::path(
    get,
    path = "/garmin/race_results_db",
    params(RaceResultsDBRequest),
    responses(RaceResultsResponse, Error)
)]
async fn race_results_db(
    query: Query<RaceResultsDBRequest>,
    _: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<RaceResultsResponse> {
    let Query(query) = query;

    let race_type = query.race_type.map_or(RaceType::Personal, Into::into);
    let mut results: Vec<_> = RaceResults::get_results_by_type(race_type, &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .map_ok(Into::into)
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;
    results.shrink_to_fit();

    Ok(JsonBase::new(results.into()).into())
}

#[derive(Serialize, Deserialize, ToSchema)]
// RaceResultsDBUpdateRequest
struct RaceResultsDBUpdateRequest {
    updates: Vec<RaceResultsWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Race Results Update",
    status = "CREATED",
    content = "text/html"
)]
#[rustfmt::skip]
struct RaceResultsUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/garmin/race_results_db",
    request_body = RaceResultsDBUpdateRequest,
    responses(RaceResultsUpdateResponse, Error)
)]
async fn race_results_db_update(
    state: State<Arc<AppState>>,
    _: LoggedUser,
    payload: Json<RaceResultsDBUpdateRequest>,
) -> WarpResult<RaceResultsUpdateResponse> {
    let Json(payload) = payload;

    let futures = payload.updates.into_iter().map(|result| {
        let pool = state.db.clone();
        let mut result: RaceResults = result.into();
        async move { result.upsert_db(&pool).await.map_err(Into::<Error>::into) }
    });
    let results: Result<Vec<()>, Error> = try_join_all(futures).await;
    results?;
    Ok(HtmlBase::new("Finished").into())
}

#[derive(UtoipaResponse)]
#[response(description = "Garmin Connect Profile")]
#[rustfmt::skip]
struct GarminConnectProfileResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/garmin/connect/profile",
    responses(GarminConnectProfileResponse, Error)
)]
async fn garmin_connect_profile(
    _: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<GarminConnectProfileResponse> {
    let mut client = GarminConnectClient::new(state.config.clone()).map_err(Into::<Error>::into)?;
    let profile = client.init().await.map_err(Into::<Error>::into)?;
    let body = garmin_connect_profile_body(profile)?.into();
    Ok(HtmlBase::new(body).into())
}

pub fn get_garmin_path(app: &AppState) -> OpenApiRouter {
    let app = Arc::new(app.clone());
    let (upload_schema, upload_paths, upload_router) = routes!(garmin_upload);
    let upload_router = upload_router.layer(DefaultBodyLimit::disable());

    OpenApiRouter::new()
        .routes(routes!(garmin))
        .routes(routes!(garmin_demo))
        .routes((upload_schema, upload_paths, upload_router))
        .routes(routes!(add_garmin_correction))
        .routes(routes!(garmin_connect_activities_db))
        .routes(routes!(garmin_connect_activities_db_update))
        .routes(routes!(garmin_sync))
        .routes(routes!(strava_sync))
        .routes(routes!(fitbit_heartrate_cache))
        .routes(routes!(fitbit_heartrate_cache_update))
        .routes(routes!(fitbit_plots))
        .routes(routes!(fitbit_plots_demo))
        .routes(routes!(heartrate_statistics_plots))
        .routes(routes!(heartrate_statistics_plots_demo))
        .routes(routes!(heartrate_plots))
        .routes(routes!(heartrate_plots_demo))
        .routes(routes!(fitbit_activities_db))
        .routes(routes!(fitbit_activities_db_update))
        .routes(routes!(heartrate_statistics_summary_db))
        .routes(routes!(heartrate_statistics_summary_db_update))
        .routes(routes!(scale_measurement))
        .routes(routes!(scale_measurement_update))
        .routes(routes!(scale_measurement_manual))
        .routes(routes!(scale_measurement_manual_input))
        .routes(routes!(strava_auth))
        .routes(routes!(strava_refresh))
        .routes(routes!(strava_callback))
        .routes(routes!(strava_activities))
        .routes(routes!(strava_athlete))
        .routes(routes!(garmin_connect_profile))
        .routes(routes!(strava_activities_db))
        .routes(routes!(strava_activities_db_update))
        .routes(routes!(strava_upload))
        .routes(routes!(strava_update))
        .routes(routes!(strava_create))
        .routes(routes!(user))
        .routes(routes!(race_result_plot))
        .routes(routes!(race_result_flag))
        .routes(routes!(race_result_import))
        .routes(routes!(race_result_plot_demo))
        .routes(routes!(race_results_db))
        .routes(routes!(race_results_db_update))
        .routes(routes!(garmin_scripts_js))
        .routes(routes!(garmin_scripts_demo_js))
        .routes(routes!(line_plot_js))
        .routes(routes!(scatter_plot_js))
        .routes(routes!(scatter_plot_with_lines_js))
        .routes(routes!(time_series_js))
        .routes(routes!(initialize_map_js))
        .with_state(app)
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Fitness Activity WebApp",
        description = "Web Frontend for Fitness Activities",
    ),
    components(schemas(LoggedUser, Pagination))
)]
pub struct ApiDoc;
