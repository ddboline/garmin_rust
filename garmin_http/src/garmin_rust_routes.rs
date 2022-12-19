#![allow(clippy::needless_pass_by_value)]
use anyhow::format_err;
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
use std::{convert::Infallible, string::ToString};
use tempdir::TempDir;
use tokio::{fs::File, io::AsyncWriteExt, task::spawn_blocking};
use tokio_stream::StreamExt;

use fitbit_lib::{
    fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate,
    fitbit_statistics_summary::FitbitStatisticsSummary, scale_measurement::ScaleMeasurement,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_config::GarminConfig,
        garmin_connect_activity::GarminConnectActivity,
        garmin_correction_lap::GarminCorrectionLap,
        garmin_file,
        garmin_summary::{get_list_of_files_from_db, GarminSummary},
        pgpool::PgPool,
        strava_activity::StravaActivity,
    },
    parsers::garmin_parse::{GarminParse, GarminParseTrait},
    utils::{date_time_wrapper::iso8601::convert_datetime_to_str, garmin_util::titlecase},
};
use garmin_reports::garmin_summary_report_txt::create_report_query;
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::StravaClient;

use crate::{
    errors::ServiceError as Error,
    garmin_elements::{
        create_fitbit_table, fitbit_body, index_new_body, strava_body, table_body, IndexConfig,
    },
    garmin_requests::{
        AddGarminCorrectionRequest, FitbitActivitiesRequest, FitbitHeartrateCacheRequest,
        FitbitHeartratePlotRequest, FitbitHeartrateUpdateRequest, FitbitStatisticsPlotRequest,
        FitbitTcxSyncRequest, GarminConnectActivitiesDBUpdateRequest, GarminHtmlRequest,
        HeartrateStatisticsSummaryDBUpdateRequest, ScaleMeasurementPlotRequest,
        ScaleMeasurementRequest, ScaleMeasurementUpdateRequest, StravaActivitiesRequest,
        StravaCreateRequest, StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
    },
    garmin_rust_app::AppState,
    logged_user::{LoggedUser, Session},
    FitbitActivityTypesWrapper, FitbitActivityWrapper, FitbitBodyWeightFatUpdateOutputWrapper,
    FitbitBodyWeightFatWrapper, FitbitHeartRateWrapper, FitbitStatisticsSummaryWrapper,
    GarminConnectActivityWrapper, RaceResultsWrapper, RaceTypeWrapper, ScaleMeasurementWrapper,
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

    let filter_iter = filter.split(',');

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
struct IndexResponse(HtmlBase<StackString, Error>);

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

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    if !session.history.contains(&grec.request.filter) {
        if session.history.len() > 5 {
            session.history.remove(0);
        }
        session.history.push(grec.request.filter.clone());
    }

    let body = get_index_body(&state.db, &state.config, &grec.request, false)
        .await
        .map_err(Into::<Error>::into)?
        .into();

    user.set_session(&state.client, &state.config, &session)
        .await
        .map_err(Into::<Error>::into)?;

    Ok(HtmlBase::new(body).into())
}

async fn get_index_body(
    pool: &PgPool,
    config: &GarminConfig,
    req: &GarminRequest,
    is_demo: bool,
) -> HttpResult<String> {
    let file_list: Vec<StackString> =
        get_list_of_files_from_db(&req.constraints.to_query_string(), pool)
            .await?
            .try_collect()
            .await?;

    match file_list.len() {
        0 => Ok(String::new()),
        1 => {
            let file_name = file_list
                .get(0)
                .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
            debug!("{}", &file_name);
            let avro_file = config.cache_dir.join(file_name.as_str());

            let gfile = if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                debug!("Cached avro file read: {:?}", &avro_file);
                g
            } else {
                let gps_file = config.gps_dir.join(file_name.as_str());
                let corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;

                debug!("Reading gps_file: {:?}", &gps_file);
                spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map)).await??
            };
            let sport: StackString = gfile.sport.into();
            let sport = titlecase(&sport);
            let dt = gfile.begin_datetime;
            let body = index_new_body(
                config.clone(),
                pool,
                format_sstr!("Garmin Event {sport} at {dt}"),
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
                config.clone(),
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
        .await
        .map_err(Into::<Error>::into)?
        .into();

    let jwt = session.get_jwt_cookie(&state.config.domain);
    let jwt_str = StackString::from_display(jwt.encoded());
    Ok(HtmlBase::new(body).with_cookie(&jwt_str).into())
}

#[derive(RwebResponse)]
#[response(description = "Javascript", content = "js")]
struct JsResponse(HtmlBase<&'static str, Infallible>);

#[get("/garmin/scripts/garmin_scripts.js")]
pub async fn garmin_scripts_js() -> WarpResult<JsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/garmin_scripts.js")).into())
}

#[get("/garmin/scripts/garmin_scripts_demo.js")]
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
) -> HttpResult<StackString> {
    let tempdir = TempDir::new("garmin")?;
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
    gcli.sync_everything(false).await?;
    gcli.proc_everything().await?;

    let query = FilterRequest {
        filter: datetimes
            .get(0)
            .map(|dt| convert_datetime_to_str((*dt).into())),
    };

    let grec = proc_pattern_wrapper(&state.config, query, &session.history, false);
    let body = get_index_body(&state.db, &state.config, &grec.request, false)
        .await?
        .into();
    Ok(body)
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

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrSyncRequest {
    pub date: DateType,
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrApiRequest {
    pub date: DateType,
}

#[derive(RwebResponse)]
#[response(description = "Garmin Sync", content = "html")]
struct GarminSyncResponse(HtmlBase<StackString, Error>);

#[get("/garmin/garmin_sync")]
pub async fn garmin_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminSyncResponse> {
    let gcli = GarminCli::from_pool(&state.db).map_err(Into::<Error>::into)?;
    let mut body = gcli
        .sync_everything(false)
        .await
        .map_err(Into::<Error>::into)?;
    body.extend_from_slice(&gcli.proc_everything().await.map_err(Into::<Error>::into)?);
    let body = body.join("\n").into();
    let body = table_body(body).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Strava Sync", content = "html")]
struct StravaSyncResponse(HtmlBase<StackString, Error>);

#[get("/garmin/strava_sync")]
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
        .map(|p| p.to_string_lossy().into_owned())
        .join("\n")
        .into();
    let body = table_body(body).into();
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
pub struct StravaCallbackRequest {
    #[schema(description = "Authorization Code")]
    pub code: StackString,
    #[schema(description = "CSRF State")]
    pub state: StackString,
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
    let alist = query
        .into_inner()
        .get_activities(&state.config)
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
    let query = query.into_inner();
    let alist = StravaActivity::read_from_db(
        &state.db,
        query.start_date.map(Into::into),
        query.end_date.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;

    Ok(JsonBase::new(alist).into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaActiviesDBUpdateRequest {
    pub updates: Vec<StravaActivityWrapper>,
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
    let updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
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
#[response(description = "Fitbit Auth", content = "html")]
struct FitbitAuthResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/auth")]
pub async fn fitbit_auth(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitAuthResponse> {
    let client = FitbitClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let url = client.get_fitbit_auth_url().map_err(Into::<Error>::into)?;
    let body = url.as_str().into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Refresh Auth", content = "html")]
struct FitbitRefreshResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/refresh_auth")]
pub async fn fitbit_refresh(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitRefreshResponse> {
    let mut client = FitbitClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let body = client
        .refresh_fitbit_access_token()
        .await
        .map_err(Into::<Error>::into)?;
    client.to_file().await.map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitHeartrateApiRequest {
    date: DateType,
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
    let query = query.into_inner();
    let client = FitbitClient::with_auth(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let hlist = client
        .get_fitbit_intraday_time_series_heartrate(query.date.into())
        .await
        .map_err(Into::<Error>::into)?
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
        .get_cache(&state.config)
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
    payload.into_inner().merge_data(&state.config).await?;
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
    let client = FitbitClient::with_auth(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let hlist = client
        .get_fitbit_bodyweightfat()
        .await
        .map_err(Into::<Error>::into)?
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
    let client = FitbitClient::with_auth(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let hlist = client
        .sync_everything(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
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
        .get_activities(&state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(hlist).into())
}

#[derive(Deserialize, Schema)]
pub struct FitbitCallbackRequest {
    #[schema(description = "Authorization Code")]
    code: StackString,
    #[schema(description = "CSRF State")]
    state: StackString,
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Callback", content = "html")]
struct FitbitCallbackResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/callback")]
pub async fn fitbit_callback(
    query: Query<FitbitCallbackRequest>,
    #[data] state: AppState,
) -> WarpResult<FitbitCallbackResponse> {
    let query = query.into_inner();
    let mut client = FitbitClient::from_file(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let body = client
        .get_fitbit_access_token(&query.code, &query.state)
        .await
        .map_err(Into::<Error>::into)?;
    client.to_file().await.map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitSyncRequest {
    date: DateType,
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Sync", content = "html")]
struct FitbitSyncResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/sync")]
pub async fn fitbit_sync(
    query: Query<FitbitSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitSyncResponse> {
    let query = query.into_inner();
    let date = query.date.into();
    let client = FitbitClient::with_auth(state.config.clone())
        .await
        .map_err(Into::<Error>::into)?;
    let mut heartrates = client
        .import_fitbit_heartrate(date)
        .await
        .map_err(Into::<Error>::into)?;
    FitbitHeartRate::calculate_summary_statistics(&client.config, &state.db, date)
        .await
        .map_err(Into::<Error>::into)?;
    let start = if heartrates.len() > 20 {
        heartrates.len() - 20
    } else {
        0
    };
    let heartrates = heartrates.split_off(start);
    let body = create_fitbit_table(heartrates).into();
    Ok(HtmlBase::new(body).into())
}

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
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        query.request.start_date.map(Into::into),
        query.request.end_date.map(Into::into),
        &state.db,
    )
    .await
    .map_err(Into::<Error>::into)?
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    let body = index_new_body(
        state.config.clone(),
        &state.db,
        "".into(),
        false,
        session.history,
        IndexConfig::HearRateSummary {
            stats,
            offset: query.request.offset,
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

    let stats: Vec<FitbitStatisticsSummary> = FitbitStatisticsSummary::read_from_db(
        query.request.start_date.map(Into::into),
        query.request.end_date.map(Into::into),
        &state.db,
    )
    .await
    .map_err(Into::<Error>::into)?
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    let body = index_new_body(
        state.config.clone(),
        &state.db,
        "".into(),
        true,
        session.history,
        IndexConfig::HearRateSummary {
            stats,
            offset: query.request.offset,
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
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let query: ScaleMeasurementPlotRequest = query.into_inner().into();

    let measurements = ScaleMeasurement::read_from_db(
        &state.db,
        query.request.start_date.map(Into::into),
        query.request.end_date.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?;

    let body = index_new_body(
        state.config.clone(),
        &state.db,
        "".into(),
        false,
        session.history,
        IndexConfig::Scale {
            measurements,
            offset: query.request.offset,
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
        query.request.start_date.map(Into::into),
        query.request.end_date.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?;

    let body = index_new_body(
        state.config.clone(),
        &state.db,
        "".into(),
        true,
        session.history,
        IndexConfig::Scale {
            measurements,
            offset: query.request.offset,
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
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;

    let heartrate = FitbitHeartRate::get_heartrate_values(
        &state.config,
        &state.db,
        query.start_date.into(),
        query.end_date.into(),
    )
    .await
    .map_err(Into::<Error>::into)?;
    let body = index_new_body(
        state.config.clone(),
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
    .await
    .map_err(Into::<Error>::into)?
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
        state.config.clone(),
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
    .await
    .map_err(Into::<Error>::into)?
    .into();

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
        .process(&state.db, &state.config)
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
        .get_measurements(&state.db)
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
    measurements.into_inner().run_update(&state.db).await?;
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
    payload.into_inner().add_corrections(&state.db).await?;
    Ok(HtmlBase::new("finised").into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Activity Types")]
struct FitbitActivityTypesResponse(JsonBase<FitbitActivityTypesWrapper, Error>);

#[get("/garmin/fitbit/fitbit_activity_types")]
pub async fn fitbit_activity_types(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitActivityTypesResponse> {
    let client = FitbitClient::with_auth(state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let result = client
        .get_fitbit_activity_types()
        .await
        .map_err(Into::<Error>::into)?;
    Ok(JsonBase::new(result.into()).into())
}

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
    let body = strava_body(result).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Fitbit Profile")]
struct FitbitProfileResponse(HtmlBase<StackString, Error>);

#[get("/garmin/fitbit/profile")]
pub async fn fitbit_profile(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<FitbitProfileResponse> {
    let client = FitbitClient::with_auth(state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let result = client
        .get_user_profile()
        .await
        .map_err(Into::<Error>::into)?;
    let body = fitbit_body(result).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Garmin Connect Activities")]
struct GarminConnectActivitiesResponse(JsonBase<Vec<GarminConnectActivityWrapper>, Error>);

#[get("/garmin/garmin_connect_activities_db")]
pub async fn garmin_connect_activities_db(
    query: Query<StravaActivitiesRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectActivitiesResponse> {
    let query = query.into_inner();

    let alist = GarminConnectActivity::read_from_db(
        &state.db,
        query.start_date.map(Into::into),
        query.end_date.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    Ok(JsonBase::new(alist).into())
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
    let body = payload
        .into_inner()
        .update(&state.db)
        .await?
        .join("\n")
        .into();
    Ok(HtmlBase::new(body).into())
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
    let query = query.into_inner();
    let alist = FitbitActivity::read_from_db(
        &state.db,
        query.start_date.map(Into::into),
        query.end_date.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?
    .into_iter()
    .map(Into::into)
    .collect();
    Ok(JsonBase::new(alist).into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct FitbitActivitiesDBUpdateRequest {
    pub updates: Vec<FitbitActivityWrapper>,
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
    let updates: Vec<_> = payload.updates.into_iter().map(Into::into).collect();
    let body = FitbitActivity::upsert_activities(&updates, &state.db)
        .await
        .map_err(Into::<Error>::into)?;
    FitbitActivity::fix_summary_id_in_db(&state.db)
        .await
        .map_err(Into::<Error>::into)?;

    let body = body.join("\n").into();
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
    let query = query.into_inner();
    let alist = FitbitStatisticsSummary::read_from_db(
        query.start_date.map(Into::into),
        query.end_date.map(Into::into),
        &state.db,
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
    Ok(JsonBase::new(alist).into())
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
pub struct RaceResultPlotRequest {
    #[schema(description = "Race Type")]
    pub race_type: RaceTypeWrapper,
    #[schema(description = "Demo Flag")]
    pub demo: Option<bool>,
}

async fn race_result_plot_impl(
    req: RaceResultPlotRequest,
    state: AppState,
    session: Session,
) -> Result<StackString, Error> {
    let model = RaceResultAnalysis::run_analysis(req.race_type.into(), &state.db).await?;
    let demo = req.demo.unwrap_or(true);

    let body = index_new_body(
        state.config.clone(),
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
    let session = user
        .get_session(&state.client, &state.config)
        .await
        .map_err(Into::<Error>::into)?;
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
pub struct RaceResultFlagRequest {
    pub id: UuidWrapper,
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
pub struct RaceResultImportRequest {
    pub filename: StackString,
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
pub struct RaceResultsDBRequest {
    #[schema(description = "Race Type")]
    pub race_type: Option<RaceTypeWrapper>,
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
    let results = RaceResults::get_results_by_type(race_type, &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .map_ok(Into::into)
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;

    Ok(JsonBase::new(results).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultsDBUpdateRequest {
    pub updates: Vec<RaceResultsWrapper>,
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
