#![allow(clippy::needless_pass_by_value)]
use futures::future::try_join_all;
use itertools::Itertools;
use log::info;
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
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_stream::StreamExt;

use fitbit_lib::{
    fitbit_client::FitbitClient, fitbit_heartrate::FitbitHeartRate,
    fitbit_statistics_summary::FitbitStatisticsSummary,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_config::GarminConfig,
        garmin_connect_activity::GarminConnectActivity,
        garmin_summary::GarminSummary,
        garmin_templates::{get_buttons, get_scripts, get_style, HBR},
        strava_activity::StravaActivity,
    },
    utils::{date_time_wrapper::iso8601::convert_datetime_to_str, garmin_util::METERS_PER_MILE},
};
use garmin_reports::garmin_file_report_html::generate_history_buttons;
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::StravaClient;

use crate::{
    errors::ServiceError as Error,
    garmin_requests::{
        get_connect_activities, AddGarminCorrectionRequest, FitbitActivitiesRequest,
        FitbitHeartrateCacheRequest, FitbitHeartratePlotRequest, FitbitHeartrateUpdateRequest,
        FitbitStatisticsPlotRequest, FitbitTcxSyncRequest, GarminConnectActivitiesDBUpdateRequest,
        GarminConnectActivitiesRequest, GarminConnectUserSummaryRequest, GarminHtmlRequest,
        HeartrateStatisticsSummaryDBUpdateRequest, ScaleMeasurementPlotRequest,
        ScaleMeasurementRequest, ScaleMeasurementUpdateRequest, StravaActivitiesRequest,
        StravaCreateRequest, StravaSyncRequest, StravaUpdateRequest, StravaUploadRequest,
    },
    garmin_rust_app::AppState,
    logged_user::{LoggedUser, Session},
    FitbitActivityTypesWrapper, FitbitActivityWrapper, FitbitBodyWeightFatUpdateOutputWrapper,
    FitbitBodyWeightFatWrapper, FitbitHeartRateWrapper, FitbitStatisticsSummaryWrapper,
    GarminConnectActivityWrapper, GarminConnectUserDailySummaryWrapper, RaceResultsWrapper,
    RaceTypeWrapper, ScaleMeasurementWrapper, StravaActivityWrapper,
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
) -> HttpResult<StackString> {
    let grec = proc_pattern_wrapper(&state.config, query, history, is_demo);
    if history.len() > 5 {
        history.remove(0);
    }
    history.push(grec.request.filter.clone());

    let body = GarminCli::from_pool(&state.db)?
        .run_html(&grec.request, grec.is_demo)
        .await?;

    Ok(body)
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
    let jwt_str = StackString::from_display(jwt.encoded());
    Ok(HtmlBase::new(body).with_cookie(&jwt_str).into())
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
    let body = GarminCli::from_pool(&state.db)?
        .run_html(&grec.request, grec.is_demo)
        .await?;

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

#[derive(RwebResponse)]
#[response(description = "Connect Sync")]
struct ConnectSyncResponse(JsonBase<Vec<String>, Error>);

#[get("/garmin/garmin_connect_sync")]
pub async fn garmin_connect_sync(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ConnectSyncResponse> {
    let body = get_connect_activities(&state.db, &state.connect_proxy)
        .await?
        .into_iter()
        .map(|x| x.to_string_lossy().into_owned())
        .collect();
    Ok(JsonBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrSyncRequest {
    pub date: DateType,
}

#[derive(RwebResponse)]
#[response(description = "Connect Sync", content = "html")]
struct ConnectHrSyncResponse(HtmlBase<StackString, Error>);

#[get("/garmin/garmin_connect_hr_sync")]
pub async fn garmin_connect_hr_sync(
    query: Query<GarminConnectHrSyncRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ConnectHrSyncResponse> {
    let query = query.into_inner();
    let mut session = state.connect_proxy.lock().await;
    session.init().await.map_err(Into::<Error>::into)?;
    let date = query.date.into();
    let heartrates = session
        .get_heartrate(date)
        .await
        .map_err(Into::<Error>::into)?;
    FitbitClient::import_garmin_connect_heartrate(state.config.clone(), &heartrates)
        .await
        .map_err(Into::<Error>::into)?;
    FitbitHeartRate::calculate_summary_statistics(&state.config, &state.db, date)
        .await
        .map_err(Into::<Error>::into)?;

    let body = heartrates.to_table(Some(20));
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrApiRequest {
    pub date: DateType,
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
    let query = query.into_inner();

    let mut session = state.connect_proxy.lock().await;
    session.init().await.map_err(Into::<Error>::into)?;

    let heartrate_data = session
        .get_heartrate(query.date.into())
        .await
        .map_err(Into::<Error>::into)?;
    let hr_vals = FitbitHeartRate::from_garmin_connect_hr(&heartrate_data)
        .into_iter()
        .map(Into::into)
        .collect();

    Ok(JsonBase::new(hr_vals).into())
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

    let body = format_sstr!(
        r#"<textarea cols=100 rows=40>{}</textarea>"#,
        body.join("\n")
    );
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
        .join("\n");
    let body = format_sstr!(r#"<textarea cols=100 rows=40>{body}</textarea>"#);
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
    .into_iter()
    .map(Into::into)
    .collect();

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
    let heartrates = client
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
    let body = FitbitHeartRate::create_table(&heartrates[start..]);
    Ok(HtmlBase::new(body).into())
}

async fn heartrate_statistics_plots_impl(
    query: FitbitStatisticsPlotRequest,
    state: AppState,
    session: Session,
) -> Result<StackString, Error> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = query.get_plots(&state.db).await?;
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
    let body = heartrate_statistics_plots_impl(query, state, session).await?;
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

    let body = heartrate_statistics_plots_impl(query, state, session).await?;
    Ok(HtmlBase::new(body).into())
}

async fn fitbit_plots_impl(
    query: ScaleMeasurementPlotRequest,
    state: AppState,
    session: Session,
) -> HttpResult<StackString> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    let mut params = query.get_plots(&state.db, &state.config).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    let body = HBR.render("GARMIN_TEMPLATE", &params)?.into();
    Ok(body)
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
) -> HttpResult<StackString> {
    let is_demo = query.is_demo;
    let buttons = get_buttons(is_demo).join("\n");
    info!("buttons {}", buttons);
    let mut params = query.get_plots(&state.db, &state.config).await?;
    params.insert(
        "HISTORYBUTTONS".into(),
        generate_history_buttons(&session.history),
    );
    params.insert("GARMIN_STYLE".into(), get_style(false));
    params.insert("GARMINBUTTONS".into(), buttons.into());
    params.insert("GARMIN_SCRIPTS".into(), get_scripts(is_demo).into());
    let body = HBR.render("GARMIN_TEMPLATE", &params)?.into();
    Ok(body)
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
    let clubs = if let Some(clubs) = &result.clubs {
        let lines = clubs
            .iter()
            .map(|c| {
                format_sstr!(
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
        format_sstr!(
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
                {lines}
                </tbody>
                </table>
            "#
        )
    } else {
        "".into()
    };
    let shoes = if let Some(shoes) = &result.shoes {
        let lines = shoes
            .iter()
            .map(|s| {
                format_sstr!(
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
        format_sstr!(
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
                <tbody>{lines}</tbody>
                </table>
            "#
        )
    } else {
        "".into()
    };
    let body = format_sstr!(
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
            format_sstr!("<tr><td>Follower Count</td><td>{follower_count}</td></tr>")
        } else {
            StackString::new()
        },
        friend_count = if let Some(friend_count) = result.friend_count {
            format_sstr!("<tr><td>Friend Count</td><td>{friend_count}</td></tr>")
        } else {
            StackString::new()
        },
        measurement_preference = if let Some(measurement_preference) = result.measurement_preference
        {
            format_sstr!(
                "<tr><td>Measurement Preference</td><td>{measurement_preference}</td></tr>"
            )
        } else {
            StackString::new()
        },
    );
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
    let body = format_sstr!(
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
        .get_activities(&state.connect_proxy)
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
    let query = query.into_inner();

    let alist = GarminConnectActivity::read_from_db(
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
#[response(description = "Garmin Connect User Summary")]
struct GarminConnectUserSummaryResponse(JsonBase<GarminConnectUserDailySummaryWrapper, Error>);

#[get("/garmin/garmin_connect_user_summary")]
pub async fn garmin_connect_user_summary(
    query: Query<GarminConnectUserSummaryRequest>,
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<GarminConnectUserSummaryResponse> {
    let js = query.into_inner().get_summary(&state.connect_proxy).await?;
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
    let is_demo = req.demo.unwrap_or(true);
    let buttons = get_buttons(is_demo).join("\n");

    let model = RaceResultAnalysis::run_analysis(req.race_type.into(), &state.db).await?;
    let demo = req.demo.unwrap_or(true);
    let mut params = model.create_plot(demo)?;

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
        .into_iter()
        .map(Into::into)
        .collect();

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
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;
    Ok(HtmlBase::new("Finished").into())
}
