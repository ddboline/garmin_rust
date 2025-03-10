#![allow(clippy::needless_pass_by_value)]
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use log::debug;
use rweb::{
    get,
    multipart::{FormData, Part},
    post, Buf, Filter, Json, Query, Rejection, Schema,
};
use rweb_helper::{
    html_response::HtmlResponse as HtmlBase, json_response::JsonResponse as JsonBase, DateType,
    RwebResponse, UuidWrapper,
};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::convert::Infallible;
use tempfile::TempDir;
use time_tz::OffsetDateTimeExt;
use tokio::{fs::File, io::AsyncWriteExt, task::spawn_blocking};
use tokio_stream::StreamExt;

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
    FitbitActivityTypesWrapper, FitbitActivityWrapper, FitbitHeartRateWrapper,
    FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper, RaceResultsWrapper,
    RaceTypeWrapper, ScaleMeasurementWrapper, StravaActivityWrapper,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

#[derive(Deserialize, Schema)]
struct FilterRequest {
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

fn optional_session() -> impl Filter<Extract = (Option<Session>,), Error = Infallible> + Copy {
    rweb::cookie::optional("session")
}

#[derive(RwebResponse)]
#[response(description = "Main Page", content = "html")]
struct IndexResponse(HtmlBase<StackString, Error>);

#[get("/garmin/index.html")]
#[openapi(description = "Main Page")]
pub async fn garmin(
    query: Query<FilterRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<IndexResponse> {
    let query = query.into_inner();

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
) -> HttpResult<String> {
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
                debug!("Cached avro file read: {:?}", &avro_file);
                g
            } else {
                let gps_file = config.gps_dir.join(file_name.as_str());
                let mut corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;
                corr_map.shrink_to_fit();

                debug!("Reading gps_file: {:?}", &gps_file);
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

#[get("/garmin/demo.html")]
#[openapi(description = "Demo Main Page")]
pub async fn garmin_demo(
    query: Query<FilterRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<IndexResponse> {
    let query = query.into_inner();

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

#[derive(RwebResponse)]
#[response(description = "Javascript", content = "js")]
struct JsResponse(HtmlBase<&'static str, Infallible>);

#[get("/garmin/scripts/garmin_scripts.js")]
#[openapi(description = "Scripts")]
pub async fn garmin_scripts_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/garmin_scripts.js")).into())
}

#[get("/garmin/scripts/garmin_scripts_demo.js")]
#[openapi(description = "Demo Scripts")]
pub async fn garmin_scripts_demo_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/garmin_scripts_demo.js")).into())
}

#[get("/garmin/scripts/line_plot.js")]
pub async fn line_plot_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/line_plot.js")).into())
}

#[get("/garmin/scripts/scatter_plot.js")]
pub async fn scatter_plot_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/scatter_plot.js")).into())
}

#[get("/garmin/scripts/scatter_plot_with_lines.js")]
pub async fn scatter_plot_with_lines_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/scatter_plot_with_lines.js")).into())
}

#[get("/garmin/scripts/time_series.js")]
pub async fn time_series_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/time_series.js")).into())
}

#[get("/garmin/scripts/initialize_map.js")]
pub async fn initialize_map_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/initialize_map.js")).into())
}

#[derive(RwebResponse)]
#[response(description = "Upload Response", content = "html", status = "CREATED")]
struct UploadResponse(HtmlBase<StackString, Error>);

#[post("/garmin/upload_file")]
pub async fn garmin_upload(
    #[filter = "rweb::multipart::form"] form: FormData,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UploadResponse> {
    let session = user.get_session(&state.client, &state.config).await?;
    let body = garmin_upload_body(form, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

async fn garmin_upload_body(
    mut form: FormData,
    state: AppState,
    session: Session,
) -> HttpResult<StackString> {
    let tempdir = TempDir::with_prefix("garmin_rust")?;
    let tempdir_str = tempdir.path().to_string_lossy();
    let mut fname = StackString::new();

    while let Some(item) = form.next().await {
        let item = item?;
        let filename = item.filename().unwrap_or("");
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

async fn save_file(file_path: &str, field: Part) -> Result<u64, Error> {
    let mut file = File::create(file_path).await?;
    let mut stream = field.stream();
    let mut buf_size = 0usize;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let chunk = chunk.chunk();
        buf_size += chunk.len();
        file.write_all(chunk).await?;
    }
    let file_size = file.metadata().await?.len();
    debug_assert!(buf_size as u64 == file_size);
    Ok(file_size)
}

#[derive(RwebResponse)]
#[response(description = "Garmin Sync", content = "html")]
struct GarminSyncResponse(HtmlBase<StackString, Error>);

#[post("/garmin/garmin_sync")]
pub async fn garmin_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminSyncResponse> {
    let gcli = GarminCli::from_pool(&state.db).map_err(Into::<Error>::into)?;
    let mut body = gcli.sync_everything().await.map_err(Into::<Error>::into)?;
    body.extend_from_slice(&gcli.proc_everything().await.map_err(Into::<Error>::into)?);
    let body = body.join("\n").into();
    let body = table_body(body)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Sync", content = "html")]
struct StravaSyncResponse(HtmlBase<StackString, Error>);

#[post("/garmin/strava_sync")]
pub async fn strava_sync(
    query: Query<StravaSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaSyncResponse> {
    let body = query
        .into_inner()
        .run_sync(&state.db, &state.config)
        .await?
        .into_iter()
        .map(|a| a.name)
        .join("\n")
        .into();
    let body = table_body(body)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Auth", content = "html")]
struct StravaAuthResponse(HtmlBase<StackString, Error>);

#[get("/garmin/strava/auth")]
pub async fn strava_auth(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaAuthResponse> {
    let client = StravaClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let body: StackString = client
        .get_authorization_url_api()
        .map_err(Into::<Error>::into)
        .map(|u| u.as_str().into())?;

    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Refresh Auth", content = "html")]
struct StravaRefreshResponse(HtmlBase<StackString, Error>);

#[get("/garmin/strava/refresh_auth")]
pub async fn strava_refresh(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
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

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "StravaCallbackRequest")]
struct StravaCallbackRequest {
    #[schema(description = "Authorization Code")]
    code: StackString,
    #[schema(description = "CSRF State")]
    state: StackString,
}

#[derive(RwebResponse)]
#[response(description = "Strava Callback", content = "html")]
struct StravaCallbackResponse(HtmlBase<StackString, Error>);

#[get("/garmin/strava/callback")]
pub async fn strava_callback(
    query: Query<StravaCallbackRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaCallbackResponse> {
    let query = query.into_inner();
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

#[derive(RwebResponse)]
#[response(description = "Strava Activities")]
struct StravaActivitiesResponse(JsonBase<Vec<StravaActivityWrapper>, Error>);

#[get("/garmin/strava/activities")]
pub async fn strava_activities(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesResponse> {
    let mut alist: Vec<_> = query
        .into_inner()
        .get_activities(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    alist.shrink_to_fit();
    Ok(JsonBase::new(alist).into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "Pagination")]
struct Pagination {
    #[schema(description = "Total Number of Entries")]
    total: usize,
    #[schema(description = "Number of Entries to Skip")]
    offset: usize,
    #[schema(description = "Number of Entries Returned")]
    limit: usize,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedStravaActivity")]
struct PaginatedStravaActivity {
    pagination: Pagination,
    data: Vec<StravaActivityWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Strava DB Activities")]
struct StravaActivitiesDBResponse(JsonBase<PaginatedStravaActivity, Error>);

#[get("/garmin/strava/activities_db")]
pub async fn strava_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesDBResponse> {
    let query = query.into_inner();

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date.map(Into::into);
    let end_date = query.end_date.map(Into::into);

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

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "StravaActiviesDBUpdateRequest")]
struct StravaActiviesDBUpdateRequest {
    updates: Vec<StravaActivityWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Strava Activities Update",
    status = "CREATED",
    content = "html"
)]
struct StravaActivitiesUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/strava/activities_db")]
pub async fn strava_activities_db_update(
    payload: Json<StravaActiviesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaActivitiesUpdateResponse> {
    let payload = payload.into_inner();
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

#[derive(RwebResponse)]
#[response(description = "Strava Upload", status = "CREATED", content = "html")]
struct StravaUploadResponse(HtmlBase<StackString, Error>);

#[post("/garmin/strava/upload")]
pub async fn strava_upload(
    payload: Json<StravaUploadRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaUploadResponse> {
    let body = payload.into_inner().run_upload(&state.config).await?;
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Update", status = "CREATED", content = "html")]
struct StravaUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/strava/update")]
pub async fn strava_update(
    payload: Json<StravaUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaUpdateResponse> {
    let body = payload.into_inner().run_update(&state.config).await?;
    Ok(HtmlBase::new(body.as_str().into()).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Create", status = "CREATED", content = "html")]
struct StravaCreateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/strava/create")]
pub async fn strava_create(
    query: Query<StravaCreateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaCreateResponse> {
    let activity_id = query
        .into_inner()
        .create_activity(&state.db, &state.config)
        .await?;
    let body = activity_id.map_or_else(|| "".into(), StackString::from_display);
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate")]
struct FitbitHeartRateResponse(JsonBase<Vec<FitbitHeartRateWrapper>, Error>);

#[get("/garmin/fitbit/heartrate_cache")]
pub async fn fitbit_heartrate_cache(
    query: Query<FitbitHeartrateCacheRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartRateResponse> {
    let mut hlist: Vec<_> = query
        .into_inner()
        .get_cache(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    hlist.shrink_to_fit();
    Ok(JsonBase::new(hlist).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Fitbit Heartrate Update",
    content = "html",
    status = "CREATED"
)]
struct FitbitHeartrateUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/fitbit/heartrate_cache")]
pub async fn fitbit_heartrate_cache_update(
    payload: Json<FitbitHeartrateUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartrateUpdateResponse> {
    let dates = payload.into_inner().merge_data(&state.config).await?;
    Ok(HtmlBase::new(format_sstr!("Finished {dates:?}")).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activities")]
struct FitbitActivitiesResponse(JsonBase<Vec<FitbitActivityWrapper>, Error>);

#[derive(RwebResponse)]
#[response(description = "Fitbit Callback", content = "html")]
struct FitbitCallbackResponse(HtmlBase<StackString, Error>);

#[derive(Serialize, Deserialize, Schema)]
struct FitbitSyncRequest {
    date: DateType,
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Sync", content = "html")]
struct FitbitSyncResponse(HtmlBase<StackString, Error>);

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate Statistics Plots", content = "html")]
struct FitbitStatisticsPlotResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/heartrate_statistics_plots")]
pub async fn heartrate_statistics_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let query: FitbitStatisticsPlotRequest = query.into_inner().into();
    let session = user.get_session(&state.client, &state.config).await?;
    let mut stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        &state.db,
        Some(query.start_date.into()),
        Some(query.end_date.into()),
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

#[get("/garmin/fitbit/heartrate_statistics_plots_demo")]
pub async fn heartrate_statistics_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<FitbitStatisticsPlotResponse> {
    let mut query: FitbitStatisticsPlotRequest = query.into_inner().into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let mut stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        &state.db,
        Some(query.start_date.into()),
        Some(query.end_date.into()),
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

#[derive(RwebResponse)]
#[response(description = "Scale Measurement Plots", content = "html")]
struct ScaleMeasurementResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/plots")]
pub async fn fitbit_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = user.get_session(&state.client, &state.config).await?;
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();

    let measurements = ScaleMeasurement::read_from_db(
        &state.db,
        Some(query.start_date.into()),
        Some(query.end_date.into()),
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

#[get("/garmin/fitbit/plots_demo")]
pub async fn fitbit_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<ScaleMeasurementResponse> {
    let session = session.unwrap_or_default();
    let mut query: ScaleMeasurementPlotRequest = query.into_inner().into();
    query.is_demo = true;

    let measurements = ScaleMeasurement::read_from_db(
        &state.db,
        Some(query.start_date.into()),
        Some(query.end_date.into()),
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

#[derive(RwebResponse)]
#[response(description = "Fitbit Heartrate Plots", content = "html")]
struct FitbitHeartratePlotResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/heartrate_plots")]
pub async fn heartrate_plots(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let query: FitbitHeartratePlotRequest = query.into_inner().into();
    let session = user.get_session(&state.client, &state.config).await?;

    let parquet_values = fitbit_archive::get_number_of_heartrate_values(
        &state.config,
        query.start_date.into(),
        query.end_date.into(),
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
            query.start_date.into(),
            query.end_date.into(),
        )
        .await
    } else {
        let config = state.config.clone();
        spawn_blocking(move || {
            fitbit_archive::get_heartrate_values(
                &config,
                query.start_date.into(),
                query.end_date.into(),
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

#[get("/garmin/fitbit/heartrate_plots_demo")]
pub async fn heartrate_plots_demo(
    query: Query<ScaleMeasurementRequest>,
    #[data] state: AppState,
    #[filter = "optional_session"] session: Option<Session>,
) -> WarpResult<FitbitHeartratePlotResponse> {
    let mut query: FitbitHeartratePlotRequest = query.into_inner().into();
    query.is_demo = true;
    let session = session.unwrap_or_default();

    let heartrate = FitbitHeartRate::get_heartrate_values(
        &state.config,
        &state.db,
        query.start_date.into(),
        query.end_date.into(),
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

#[derive(RwebResponse)]
#[response(description = "Fitbit Tcx Sync")]
struct FitbitTcxSyncResponse(JsonBase<Vec<String>, Error>);

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedScaleMeasurement")]
struct PaginatedScaleMeasurement {
    pagination: Pagination,
    data: Vec<ScaleMeasurementWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Scale Measurements")]
struct ScaleMeasurementsResponse(JsonBase<PaginatedScaleMeasurement, Error>);

#[get("/garmin/scale_measurements")]
pub async fn scale_measurement(
    query: Query<ScaleMeasurementRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementsResponse> {
    let query = query.into_inner();

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date.map(Into::into);
    let end_date = query.end_date.map(Into::into);

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
        query.start_date.map(Into::into),
        query.end_date.map(Into::into),
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
    let measurements = measurements.into_inner();
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

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "ScaleMeasurementManualRequest")]
struct ScaleMeasurementManualRequest {
    #[schema(description = "Weight in lbs", example = r#""189.0""#)]
    weight_in_lbs: f64,
    #[schema(description = "Body fat percent", example = r#""20.3""#)]
    body_fat_percent: f64,
    #[schema(description = "Muscle mass in lbs", example = r#""153.0""#)]
    muscle_mass_lbs: f64,
    #[schema(description = "Body water percent", example = r#""63.0""#)]
    body_water_percent: f64,
    #[schema(description = "Bone mass in lbs", example = r#""63.0""#)]
    bone_mass_lbs: f64,
}

#[derive(RwebResponse)]
#[response(
    description = "Scale Measurement Manual Input Post",
    status = "CREATED"
)]
struct ScaleMeasurementManualResponse(JsonBase<ScaleMeasurementWrapper, Error>);

#[post("/garmin/scale_measurements/manual")]
pub async fn scale_measurement_manual(
    payload: Json<ScaleMeasurementManualRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ScaleMeasurementManualResponse> {
    let payload = payload.into_inner();
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
    if let Ok(mut client) = GarminConnectClient::new(state.config) {
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

#[derive(RwebResponse)]
#[response(description = "Scale Measurement Manual Input")]
struct ScaleMeasurementManualInputResponse(HtmlBase<StackString, Error>);

#[post("/garmin/scale_measurements/manual/input")]
pub async fn scale_measurement_manual_input(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
) -> WarpResult<ScaleMeasurementManualInputResponse> {
    let body = scale_measurement_manual_input_body()?;
    Ok(HtmlBase::new(body.into()).into())
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
#[response(description = "Add correction", content = "html", status = "CREATED")]
struct AddGarminCorrectionResponse(HtmlBase<&'static str, Error>);

#[post("/garmin/add_garmin_correction")]
pub async fn add_garmin_correction(
    payload: Json<AddGarminCorrectionRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<AddGarminCorrectionResponse> {
    payload.into_inner().add_corrections(&state.db).await?;
    Ok(HtmlBase::new("finised").into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activity Types")]
struct FitbitActivityTypesResponse(JsonBase<FitbitActivityTypesWrapper, Error>);

#[derive(RwebResponse)]
#[response(description = "Strava Athlete")]
struct StravaAthleteResponse(HtmlBase<StackString, Error>);

#[get("/garmin/strava/athlete")]
pub async fn strava_athlete(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<StravaAthleteResponse> {
    let client = StravaClient::with_auth(state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let result = client
        .get_strava_athlete()
        .await
        .map_err(Into::<Error>::into)?;
    let body = strava_body(result)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Profile")]
struct FitbitProfileResponse(HtmlBase<StackString, Error>);

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedGarminConnectActivity")]
struct PaginatedGarminConnectActivity {
    pagination: Pagination,
    data: Vec<GarminConnectActivityWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Garmin Connect Activities")]
struct GarminConnectActivitiesResponse(JsonBase<PaginatedGarminConnectActivity, Error>);

#[get("/garmin/garmin_connect_activities_db")]
pub async fn garmin_connect_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesResponse> {
    let query = query.into_inner();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date.map(Into::into);
    let end_date = query.end_date.map(Into::into);

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

#[derive(RwebResponse)]
#[response(
    description = "Garmin Connect Activities",
    content = "html",
    status = "CREATED"
)]
struct GarminConnectActivitiesUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/garmin_connect_activities_db")]
pub async fn garmin_connect_activities_db_update(
    payload: Json<GarminConnectActivitiesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesUpdateResponse> {
    let payload = payload.into_inner();
    let mut updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
    updates.shrink_to_fit();
    let body: StackString = GarminConnectActivity::upsert_activities(&updates, &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .join("\n")
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedFitbitActivity")]
struct PaginatedFitbitActivity {
    pagination: Pagination,
    data: Vec<FitbitActivityWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activities")]
struct FitbitActivitiesDBResponse(JsonBase<PaginatedFitbitActivity, Error>);

#[get("/garmin/fitbit/fitbit_activities_db")]
pub async fn fitbit_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivitiesDBResponse> {
    let query = query.into_inner();

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date.map(Into::into);
    let end_date = query.end_date.map(Into::into);

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

#[derive(Debug, Serialize, Deserialize, Schema)]
struct FitbitActivitiesDBUpdateRequest {
    updates: Vec<FitbitActivityWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Fitbit Activities Update",
    content = "html",
    status = "CREATED"
)]
struct FitbitActivitiesDBUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/fitbit/fitbit_activities_db")]
pub async fn fitbit_activities_db_update(
    payload: Json<FitbitActivitiesDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivitiesDBUpdateResponse> {
    let payload = payload.into_inner();
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

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedFitbitStatisticsSummary")]
struct PaginatedFitbitStatisticsSummary {
    pagination: Pagination,
    data: Vec<FitbitStatisticsSummaryWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Heartrate Statistics")]
struct HeartrateStatisticsResponse(JsonBase<PaginatedFitbitStatisticsSummary, Error>);

#[get("/garmin/fitbit/heartrate_statistics_summary_db")]
pub async fn heartrate_statistics_summary_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<HeartrateStatisticsResponse> {
    let query = query.into_inner();

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_date = query.start_date.map(Into::into);
    let end_date = query.end_date.map(Into::into);

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

#[derive(RwebResponse)]
#[response(
    description = "Heartrate Statistics Update",
    content = "html",
    status = "CREATED"
)]
struct HeartrateStatisticsUpdateResponse(HtmlBase<StackString, Error>);

#[post("/garmin/fitbit/heartrate_statistics_summary_db")]
pub async fn heartrate_statistics_summary_db_update(
    payload: Json<HeartrateStatisticsSummaryDBUpdateRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<HeartrateStatisticsUpdateResponse> {
    let body = payload
        .into_inner()
        .process_updates(&state.db)
        .await?
        .join("\n")
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
#[schema(component = "RaceResultPlotRequest")]
struct RaceResultPlotRequest {
    #[schema(description = "Race Type")]
    race_type: RaceTypeWrapper,
    #[schema(description = "Demo Flag")]
    demo: Option<bool>,
}

async fn race_result_plot_impl(
    req: RaceResultPlotRequest,
    state: AppState,
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

#[derive(RwebResponse)]
#[response(description = "Race Result Plot", content = "html")]
struct RaceResultPlotResponse(HtmlBase<StackString, Error>);

#[get("/garmin/race_result_plot")]
pub async fn race_result_plot(
    query: Query<RaceResultPlotRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultPlotResponse> {
    let mut query = query.into_inner();
    query.demo = Some(false);
    let session = user.get_session(&state.client, &state.config).await?;
    let body = race_result_plot_impl(query, state, session).await?;
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
    let body = race_result_plot_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
struct RaceResultFlagRequest {
    id: UuidWrapper,
}

#[derive(RwebResponse)]
#[response(description = "Race Result Plot", content = "html")]
struct RaceResultFlagResponse(HtmlBase<StackString, Error>);

#[get("/garmin/race_result_flag")]
pub async fn race_result_flag(
    query: Query<RaceResultFlagRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<RaceResultFlagResponse> {
    let query = query.into_inner();

    let result = if let Some(mut result) = RaceResults::get_result_by_id(query.id.into(), &state.db)
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

#[derive(Serialize, Deserialize, Schema)]
struct RaceResultImportRequest {
    filename: StackString,
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
    let query = query.into_inner();

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

#[derive(Serialize, Deserialize, Schema)]
struct RaceResultsDBRequest {
    #[schema(description = "Race Type")]
    race_type: Option<RaceTypeWrapper>,
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
    let query = query.into_inner();

    let race_type = query.race_type.map_or(RaceType::Personal, Into::into);
    let mut results: Vec<_> = RaceResults::get_results_by_type(race_type, &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .map_ok(Into::into)
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;
    results.shrink_to_fit();

    Ok(JsonBase::new(results).into())
}

#[derive(Serialize, Deserialize, Schema)]
#[schema(component = "RaceResultsDBUpdateRequest")]
struct RaceResultsDBUpdateRequest {
    updates: Vec<RaceResultsWrapper>,
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
    let payload = payload.into_inner();

    let futures = payload.updates.into_iter().map(|result| {
        let pool = state.db.clone();
        let mut result: RaceResults = result.into();
        async move { result.upsert_db(&pool).await.map_err(Into::<Error>::into) }
    });
    let results: Result<Vec<()>, Error> = try_join_all(futures).await;
    results?;
    Ok(HtmlBase::new("Finished").into())
}

#[derive(RwebResponse)]
#[response(description = "Garmin Connect Profile")]
struct GarminConnectProfileResponse(HtmlBase<StackString, Error>);

#[get("/garmin/connect/profile")]
pub async fn garmin_connect_profile(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectProfileResponse> {
    let mut client = GarminConnectClient::new(state.config).map_err(Into::<Error>::into)?;
    let profile = client.init().await.map_err(Into::<Error>::into)?;
    let body = garmin_connect_profile_body(profile)?.into();
    Ok(HtmlBase::new(body).into())
}
