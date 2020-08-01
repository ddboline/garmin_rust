use anyhow::format_err;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{fs::remove_file, sync::Mutex, task::spawn_blocking};

use fitbit_lib::{
    fitbit_client::{FitbitBodyWeightFatUpdateOutput, FitbitClient, FitbitUserProfile},
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_connect_lib::{
    garmin_connect_client::{get_garmin_connect_session, GarminConnectUserDailySummary},
    garmin_connect_proxy::GarminConnectProxy,
};
use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_connect_activity::GarminConnectActivity,
        garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionMap},
        garmin_summary::{get_filename_from_datetime, get_list_of_files_from_db, GarminSummary},
        pgpool::PgPool,
        strava_activity::StravaActivity,
    },
    utils::sport_types::SportTypes,
};
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::{StravaAthlete, StravaClient};

use crate::{errors::ServiceError as Error, CONFIG};

lazy_static! {
    static ref CONNECT_PROXY: Mutex<GarminConnectProxy> = Mutex::new(GarminConnectProxy::default());
}

pub async fn close_connect_proxy() -> Result<(), Error> {
    let mut proxy = CONNECT_PROXY.lock().await;
    if proxy.last_used < Utc::now() - Duration::seconds(300) {
        proxy.close().await?;
    }
    Ok(())
}

#[async_trait]
pub trait HandleRequest<T> {
    type Result;
    async fn handle(&self, req: T) -> Self::Result;
}

pub struct GarminCorrRequest {}

#[async_trait]
impl HandleRequest<GarminCorrRequest> for PgPool {
    type Result = Result<GarminCorrectionMap, Error>;
    async fn handle(&self, _: GarminCorrRequest) -> Self::Result {
        GarminCorrectionLap::read_corrections_from_db(self)
            .await
            .map_err(Into::into)
    }
}

pub struct GarminHtmlRequest {
    pub request: GarminRequest,
    pub is_demo: bool,
}

#[async_trait]
impl HandleRequest<GarminHtmlRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: GarminHtmlRequest) -> Self::Result {
        let body = GarminCli::from_pool(&self)?
            .run_html(&msg.request, msg.is_demo)
            .await?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.request.constraints.join(" OR "), &pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Default)]
pub struct GarminListRequest {
    pub constraints: Vec<StackString>,
}

impl Into<GarminListRequest> for GarminHtmlRequest {
    fn into(self) -> GarminListRequest {
        GarminListRequest {
            constraints: self.request.constraints,
        }
    }
}

impl GarminListRequest {
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.constraints.join(" OR "), &pool)
            .await
            .map_err(Into::into)
    }
}

#[async_trait]
impl HandleRequest<GarminListRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, msg: GarminListRequest) -> Self::Result {
        msg.get_list_of_files_from_db(self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminUploadRequest {
    pub filename: PathBuf,
}

#[async_trait]
impl HandleRequest<GarminUploadRequest> for PgPool {
    type Result = Result<Vec<PathBuf>, Error>;
    async fn handle(&self, req: GarminUploadRequest) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let filenames = vec![req.filename];
        gcli.process_filenames(&filenames).await?;
        gcli.proc_everything().await?;
        Ok(filenames)
    }
}

pub struct GarminConnectSyncRequest {}

#[async_trait]
impl HandleRequest<GarminConnectSyncRequest> for PgPool {
    type Result = Result<Vec<PathBuf>, Error>;
    async fn handle(&self, _: GarminConnectSyncRequest) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;

        let max_timestamp = Utc::now() - Duration::days(30);

        // let session = get_garmin_connect_session(&CONFIG).await?;
        let mut session = CONNECT_PROXY.lock().await;
        session.init(CONFIG.clone()).await?;

        let new_activities: Vec<_> = session.get_activities(max_timestamp).await?;
        let new_activities =
            GarminConnectActivity::merge_new_activities(new_activities, self).await?;

        let filenames = session.get_activity_files(&new_activities).await?;
        if !filenames.is_empty() {
            gcli.process_filenames(&filenames).await?;
            gcli.proc_everything().await?;
        }

        Ok(filenames)
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrSyncRequest {
    pub date: NaiveDate,
}

#[async_trait]
impl HandleRequest<GarminConnectHrSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    async fn handle(&self, req: GarminConnectHrSyncRequest) -> Self::Result {
        // let session = get_garmin_connect_session(&CONFIG).await?;
        let mut session = CONNECT_PROXY.lock().await;
        session.init(CONFIG.clone()).await?;

        let heartrate_data = session.get_heartrate(req.date).await?;
        FitbitClient::import_garmin_connect_heartrate(CONFIG.clone(), &heartrate_data).await?;
        let config = CONFIG.clone();
        FitbitHeartRate::calculate_summary_statistics(&config, &self, req.date)
            .await
            .map_err(Into::into)
            .map(|_| ())
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrApiRequest {
    pub date: NaiveDate,
}

#[async_trait]
impl HandleRequest<GarminConnectHrApiRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    async fn handle(&self, req: GarminConnectHrApiRequest) -> Self::Result {
        // let session = get_garmin_connect_session(&CONFIG).await?;
        let mut session = CONNECT_PROXY.lock().await;
        session.init(CONFIG.clone()).await?;

        let heartrate_data = session.get_heartrate(req.date).await?;
        let hr_vals = FitbitHeartRate::from_garmin_connect_hr(&heartrate_data);
        Ok(hr_vals)
    }
}

pub struct StravaSyncRequest {}

#[async_trait]
impl HandleRequest<StravaSyncRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, _: StravaSyncRequest) -> Self::Result {
        let config = CONFIG.clone();
        let pool = PgPool::new(&config.pgurl);

        let max_datetime = Utc::now() - Duration::days(15);

        let client = StravaClient::with_auth(config).await?;
        let new_activities: Vec<_> = client
            .get_all_strava_activites(Some(max_datetime), None)
            .await?;

        let output = StravaActivity::upsert_activities(&new_activities, &pool).await?;

        Ok(output)
    }
}

pub struct GarminSyncRequest {}

#[async_trait]
impl HandleRequest<GarminSyncRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, _: GarminSyncRequest) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let mut output = gcli.sync_everything(false).await?;
        output.extend_from_slice(&gcli.proc_everything().await?);
        Ok(output)
    }
}

pub struct FitbitAuthRequest {}

#[async_trait]
impl HandleRequest<FitbitAuthRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, _: FitbitAuthRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::from_file(config).await?;
        let url = client.get_fitbit_auth_url().await?;
        Ok(url.as_str().into())
    }
}

pub struct FitbitRefreshRequest {}

#[async_trait]
impl HandleRequest<FitbitRefreshRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, _: FitbitRefreshRequest) -> Self::Result {
        let config = CONFIG.clone();
        let mut client = FitbitClient::from_file(config).await?;
        let body = client.refresh_fitbit_access_token().await?;
        client.to_file().await?;
        Ok(body)
    }
}

#[derive(Deserialize)]
pub struct FitbitCallbackRequest {
    code: StackString,
    state: StackString,
}

#[async_trait]
impl HandleRequest<FitbitCallbackRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: FitbitCallbackRequest) -> Self::Result {
        let config = CONFIG.clone();
        let mut client = FitbitClient::from_file(config).await?;
        let body = client
            .get_fitbit_access_token(&msg.code, &msg.state)
            .await?;
        client.to_file().await?;
        Ok(body)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateApiRequest {
    date: NaiveDate,
}

#[async_trait]
impl HandleRequest<FitbitHeartrateApiRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    async fn handle(&self, msg: FitbitHeartrateApiRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client
            .get_fitbit_intraday_time_series_heartrate(msg.date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateCacheRequest {
    date: NaiveDate,
}

#[async_trait]
impl HandleRequest<FitbitHeartrateCacheRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    async fn handle(&self, msg: FitbitHeartrateCacheRequest) -> Self::Result {
        let config = CONFIG.clone();
        spawn_blocking(move || {
            FitbitHeartRate::read_avro_by_date(&config, msg.date).map_err(Into::into)
        })
        .await?
    }
}

pub struct FitbitBodyWeightFatRequest {}

#[async_trait]
impl HandleRequest<FitbitBodyWeightFatRequest> for PgPool {
    type Result = Result<Vec<FitbitBodyWeightFat>, Error>;
    async fn handle(&self, _: FitbitBodyWeightFatRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_fitbit_bodyweightfat().await.map_err(Into::into)
    }
}

pub struct FitbitBodyWeightFatUpdateRequest {}

#[async_trait]
impl HandleRequest<FitbitBodyWeightFatUpdateRequest> for PgPool {
    type Result = Result<FitbitBodyWeightFatUpdateOutput, Error>;
    async fn handle(&self, _: FitbitBodyWeightFatUpdateRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.sync_everything(self).await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitSyncRequest {
    date: NaiveDate,
}

#[async_trait]
impl HandleRequest<FitbitSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    async fn handle(&self, msg: FitbitSyncRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.import_fitbit_heartrate(msg.date).await?;
        FitbitHeartRate::calculate_summary_statistics(&client.config, &self, msg.date)
            .await
            .map_err(Into::into)
            .map(|_| ())
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitTcxSyncRequest {
    pub start_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<FitbitTcxSyncRequest> for PgPool {
    type Result = Result<Vec<PathBuf>, Error>;
    async fn handle(&self, msg: FitbitTcxSyncRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = Arc::new(FitbitClient::with_auth(config).await?);
        let start_date = msg
            .start_date
            .unwrap_or_else(|| (Utc::now() - Duration::days(10)).naive_utc().date());

        #[allow(clippy::filter_map)]
        let futures = client
            .get_tcx_urls(start_date)
            .await?
            .into_iter()
            .filter_map(|(start_time, tcx_url)| {
                let fname = client
                    .config
                    .gps_dir
                    .join(start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string())
                    .with_extension("tcx");
                if fname.exists() {
                    None
                } else {
                    Some((fname, tcx_url))
                }
            })
            .map(|(fname, tcx_url)| {
                let client = client.clone();
                async move {
                    let data = client.download_tcx(&tcx_url).await?;
                    tokio::fs::write(&fname, &data).await?;
                    Ok(fname)
                }
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;

        let gcli = GarminCli::from_pool(&self)?;
        gcli.proc_everything().await?;
        results
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScaleMeasurementRequest {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub button_date: Option<NaiveDate>,
}

impl ScaleMeasurementRequest {
    fn add_default(&self, ndays: i64) -> Self {
        Self {
            start_date: match self.start_date {
                Some(d) => Some(d),
                None => Some((Local::now() - Duration::days(ndays)).naive_local().date()),
            },
            end_date: match self.end_date {
                Some(d) => Some(d),
                None => Some(Local::now().naive_local().date()),
            },
            button_date: match self.button_date {
                Some(d) => Some(d),
                None => Some(Local::now().naive_local().date()),
            },
        }
    }
}

#[async_trait]
impl HandleRequest<ScaleMeasurementRequest> for PgPool {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
    async fn handle(&self, req: ScaleMeasurementRequest) -> Self::Result {
        ScaleMeasurement::read_from_db(self, req.start_date, req.end_date)
            .await
            .map_err(Into::into)
    }
}

pub struct FitbitStatisticsPlotRequest {
    pub request: ScaleMeasurementRequest,
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for FitbitStatisticsPlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(365);
        Self {
            request: item,
            is_demo: false,
        }
    }
}

#[async_trait]
impl HandleRequest<FitbitStatisticsPlotRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, req: FitbitStatisticsPlotRequest) -> Self::Result {
        let stats = FitbitStatisticsSummary::read_from_db(
            req.request.start_date,
            req.request.end_date,
            self,
        )
        .await?;
        FitbitStatisticsSummary::get_fitbit_statistics_plots(&stats, req.is_demo)
            .map_err(Into::into)
    }
}

pub struct ScaleMeasurementPlotRequest {
    pub request: ScaleMeasurementRequest,
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for ScaleMeasurementPlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(365);
        Self {
            request: item,
            is_demo: false,
        }
    }
}

#[async_trait]
impl HandleRequest<ScaleMeasurementPlotRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, req: ScaleMeasurementPlotRequest) -> Self::Result {
        let measurements =
            ScaleMeasurement::read_from_db(self, req.request.start_date, req.request.end_date)
                .await?;
        ScaleMeasurement::get_scale_measurement_plots(&measurements, req.is_demo)
            .map_err(Into::into)
    }
}

pub struct FitbitHeartratePlotRequest {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub button_date: Option<NaiveDate>,
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for FitbitHeartratePlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(3);
        Self {
            start_date: item.start_date.expect("this should be impossible"),
            end_date: item.end_date.expect("this should be impossible"),
            button_date: item.button_date,
            is_demo: false,
        }
    }
}

#[async_trait]
impl HandleRequest<FitbitHeartratePlotRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, req: FitbitHeartratePlotRequest) -> Self::Result {
        let config = CONFIG.clone();
        FitbitHeartRate::get_heartrate_plot(
            &config,
            self,
            req.start_date,
            req.end_date,
            req.button_date,
            req.is_demo,
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScaleMeasurementUpdateRequest {
    pub measurements: Vec<ScaleMeasurement>,
}

#[async_trait]
impl HandleRequest<ScaleMeasurementUpdateRequest> for PgPool {
    type Result = Result<(), Error>;
    async fn handle(&self, msg: ScaleMeasurementUpdateRequest) -> Self::Result {
        let measurement_set: HashSet<_> = ScaleMeasurement::read_from_db(self, None, None)
            .await?
            .into_par_iter()
            .map(|d| d.datetime)
            .collect();
        let measurement_set = Arc::new(measurement_set);
        let futures = msg.measurements.into_iter().map(|meas| {
            let measurement_set = measurement_set.clone();
            async move {
                if measurement_set.contains(&meas.datetime) {
                    debug!("measurement exists {:?}", meas);
                } else {
                    meas.insert_into_db(self).await?;
                    debug!("measurement inserted {:?}", meas);
                }
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        Ok(())
    }
}

pub struct StravaAuthRequest {}

#[async_trait]
impl HandleRequest<StravaAuthRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, _: StravaAuthRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = StravaClient::from_file(config).await?;
        client
            .get_authorization_url_api()
            .await
            .map_err(Into::into)
            .map(|u| u.as_str().into())
    }
}

pub struct StravaRefreshRequest {}

#[async_trait]
impl HandleRequest<StravaRefreshRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, _: StravaRefreshRequest) -> Self::Result {
        let config = CONFIG.clone();
        let mut client = StravaClient::from_file(config).await?;
        client.refresh_access_token().await?;
        client.to_file().await?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.into())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

#[async_trait]
impl HandleRequest<StravaCallbackRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: StravaCallbackRequest) -> Self::Result {
        let config = CONFIG.clone();
        let mut client = StravaClient::from_file(config).await?;
        client.process_callback(&msg.code, &msg.state).await?;
        client.to_file().await?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.into())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaActivitiesRequest {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<StravaActivitiesRequest> for PgPool {
    type Result = Result<Vec<StravaActivity>, Error>;
    async fn handle(&self, msg: StravaActivitiesRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = StravaClient::with_auth(config).await?;
        let start_date = msg
            .start_date
            .map(|s| DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(0, 0, 0)), Utc));
        let end_date = msg.end_date.map(|s| {
            DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(23, 59, 59)), Utc)
        });
        client
            .get_all_strava_activites(start_date, end_date)
            .await
            .map_err(Into::into)
    }
}

pub struct StravaActivitiesDBRequest(pub StravaActivitiesRequest);

#[async_trait]
impl HandleRequest<StravaActivitiesDBRequest> for PgPool {
    type Result = Result<Vec<StravaActivity>, Error>;
    async fn handle(&self, msg: StravaActivitiesDBRequest) -> Self::Result {
        StravaActivity::read_from_db(self, msg.0.start_date, msg.0.end_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaActiviesDBUpdateRequest {
    pub updates: Vec<StravaActivity>,
}

#[async_trait]
impl HandleRequest<StravaActiviesDBUpdateRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, msg: StravaActiviesDBUpdateRequest) -> Self::Result {
        StravaActivity::upsert_activities(&msg.updates, self)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaUploadRequest {
    pub filename: StackString,
    pub title: StackString,
    pub activity_type: StackString,
    pub description: Option<StackString>,
    pub is_private: Option<bool>,
}

#[async_trait]
impl HandleRequest<StravaUploadRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: StravaUploadRequest) -> Self::Result {
        if !Path::new(msg.filename.as_str()).exists() {
            return Ok(format!("File {} does not exist", msg.filename).into());
        }
        let config = CONFIG.clone();
        let client = StravaClient::with_auth(config).await?;
        client
            .upload_strava_activity(
                &Path::new(msg.filename.as_str()),
                &msg.title,
                msg.description.as_ref().map_or("", StackString::as_str),
            )
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaUpdateRequest {
    pub activity_id: u64,
    pub title: StackString,
    pub activity_type: StackString,
    pub description: Option<StackString>,
    pub is_private: Option<bool>,
}

#[async_trait]
impl HandleRequest<StravaUpdateRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: StravaUpdateRequest) -> Self::Result {
        let sport = msg.activity_type.parse()?;

        let config = CONFIG.clone();
        let client = StravaClient::with_auth(config).await?;
        client
            .update_strava_activity(
                msg.activity_id,
                &msg.title,
                msg.description.as_ref().map(StackString::as_str),
                sport,
            )
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaCreateRequest {
    pub filename: StackString,
}

#[async_trait]
impl HandleRequest<StravaCreateRequest> for PgPool {
    type Result = Result<Option<i64>, Error>;
    async fn handle(&self, msg: StravaCreateRequest) -> Self::Result {
        if let Some(gfile) = GarminSummary::get_by_filename(self, msg.filename.as_str()).await? {
            let mut strava_activity: StravaActivity = gfile.into();
            let config = CONFIG.clone();
            let client = StravaClient::with_auth(config).await?;
            let activity_id = client.create_strava_activity(&strava_activity).await?;
            strava_activity.id = activity_id;
            strava_activity.insert_into_db(self).await?;
            Ok(Some(activity_id))
        } else {
            Ok(None)
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AddGarminCorrectionRequest {
    pub start_time: DateTime<Utc>,
    pub lap_number: i32,
    pub distance: Option<f64>,
    pub duration: Option<f64>,
    pub sport: Option<SportTypes>,
}

#[async_trait]
impl HandleRequest<AddGarminCorrectionRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, msg: AddGarminCorrectionRequest) -> Self::Result {
        let mut corr_map = self.handle(GarminCorrRequest {}).await?;
        let filename = get_filename_from_datetime(self, msg.start_time)
            .await?
            .ok_or_else(|| {
                format_err!(
                    "start_time {} doesn't match any existing file",
                    msg.start_time
                )
            })?;
        let unique_key = (msg.start_time, msg.lap_number);

        let mut new_corr = if let Some(corr) = corr_map.get(&unique_key) {
            *corr
        } else {
            GarminCorrectionLap::new()
                .with_start_time(msg.start_time)
                .with_lap_number(msg.lap_number)
        };

        if msg.distance.is_some() {
            new_corr.distance = msg.distance;
        }
        if msg.duration.is_some() {
            new_corr.duration = msg.duration;
        }
        if msg.sport.is_some() {
            new_corr.sport = msg.sport;
        }

        corr_map.insert(unique_key, new_corr);

        GarminCorrectionLap::dump_corrections_to_db(&corr_map, self).await?;

        let cache_path = CONFIG.cache_dir.join(&format!("{}.avro", filename));
        let summary_path = CONFIG
            .summary_cache
            .join(&format!("{}.summary.avro", filename));
        remove_file(cache_path).await?;
        remove_file(summary_path).await?;

        let gcli = GarminCli::from_pool(&self)?;
        gcli.proc_everything().await?;

        Ok("".into())
    }
}

pub struct FitbitActivityTypesRequest {}

#[async_trait]
impl HandleRequest<FitbitActivityTypesRequest> for PgPool {
    type Result = Result<HashMap<u64, StackString>, Error>;
    async fn handle(&self, _: FitbitActivityTypesRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_fitbit_activity_types().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitActivitiesRequest {
    pub start_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<FitbitActivitiesRequest> for PgPool {
    type Result = Result<Vec<FitbitActivity>, Error>;
    async fn handle(&self, req: FitbitActivitiesRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        let start_date = req
            .start_date
            .unwrap_or_else(|| (Utc::now() - Duration::days(14)).naive_local().date());
        client
            .get_all_activities(start_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminConnectActivitiesRequest {
    pub start_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<GarminConnectActivitiesRequest> for PgPool {
    type Result = Result<Vec<GarminConnectActivity>, Error>;
    async fn handle(&self, req: GarminConnectActivitiesRequest) -> Self::Result {
        let start_date = req
            .start_date
            .unwrap_or_else(|| (Utc::now() - Duration::days(14)).naive_local().date());
        let start_datetime = DateTime::from_utc(
            NaiveDateTime::new(start_date, NaiveTime::from_hms(0, 0, 0)),
            Utc,
        );
        // let session = get_garmin_connect_session(&CONFIG).await?;
        let mut session = CONNECT_PROXY.lock().await;
        session.init(CONFIG.clone()).await?;

        session
            .get_activities(start_datetime)
            .await
            .map_err(Into::into)
    }
}

pub struct StravaAthleteRequest {}

#[async_trait]
impl HandleRequest<StravaAthleteRequest> for PgPool {
    type Result = Result<StravaAthlete, Error>;
    async fn handle(&self, _: StravaAthleteRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = StravaClient::with_auth(config).await?;
        client.get_strava_athlete().await.map_err(Into::into)
    }
}

pub struct FitbitProfileRequest {}

#[async_trait]
impl HandleRequest<FitbitProfileRequest> for PgPool {
    type Result = Result<FitbitUserProfile, Error>;
    async fn handle(&self, _: FitbitProfileRequest) -> Self::Result {
        let config = CONFIG.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_user_profile().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminConnectUserSummaryRequest {
    pub date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<GarminConnectUserSummaryRequest> for PgPool {
    type Result = Result<GarminConnectUserDailySummary, Error>;
    async fn handle(&self, msg: GarminConnectUserSummaryRequest) -> Self::Result {
        // let session = get_garmin_connect_session(&config).await?;
        let mut session = CONNECT_PROXY.lock().await;
        session.init(CONFIG.clone()).await?;

        let date = msg
            .date
            .unwrap_or_else(|| Local::now().naive_local().date());
        session.get_user_summary(date).await.map_err(Into::into)
    }
}

pub struct GarminConnectActivitiesDBRequest(pub StravaActivitiesRequest);

#[async_trait]
impl HandleRequest<GarminConnectActivitiesDBRequest> for PgPool {
    type Result = Result<Vec<GarminConnectActivity>, Error>;
    async fn handle(&self, msg: GarminConnectActivitiesDBRequest) -> Self::Result {
        GarminConnectActivity::read_from_db(self, msg.0.start_date, msg.0.end_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GarminConnectActivitiesDBUpdateRequest {
    pub updates: Vec<GarminConnectActivity>,
}

#[async_trait]
impl HandleRequest<GarminConnectActivitiesDBUpdateRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, msg: GarminConnectActivitiesDBUpdateRequest) -> Self::Result {
        GarminConnectActivity::upsert_activities(&msg.updates, self)
            .await
            .map_err(Into::into)
    }
}

pub struct FitbitActivitiesDBRequest(pub StravaActivitiesRequest);

#[async_trait]
impl HandleRequest<FitbitActivitiesDBRequest> for PgPool {
    type Result = Result<Vec<FitbitActivity>, Error>;
    async fn handle(&self, msg: FitbitActivitiesDBRequest) -> Self::Result {
        FitbitActivity::read_from_db(self, msg.0.start_date, msg.0.end_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FitbitActivitiesDBUpdateRequest {
    pub updates: Vec<FitbitActivity>,
}

#[async_trait]
impl HandleRequest<FitbitActivitiesDBUpdateRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
    async fn handle(&self, msg: FitbitActivitiesDBUpdateRequest) -> Self::Result {
        FitbitActivity::upsert_activities(&msg.updates, self)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct RaceResultPlotRequest {
    pub race_type: RaceType,
    pub demo: Option<bool>,
}

#[async_trait]
impl HandleRequest<RaceResultPlotRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, req: RaceResultPlotRequest) -> Self::Result {
        let model = RaceResultAnalysis::run_analysis(req.race_type, self).await?;
        let demo = req.demo.unwrap_or(true);
        model.create_plot(demo).map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize)]
pub struct RaceResultFlagRequest {
    pub id: i32,
}

#[async_trait]
impl HandleRequest<RaceResultFlagRequest> for PgPool {
    type Result = Result<StackString, Error>;
    async fn handle(&self, req: RaceResultFlagRequest) -> Self::Result {
        if let Some(mut result) = RaceResults::get_result_by_id(req.id, self).await? {
            result.race_flag = !result.race_flag;
            result.update_db(self).await?;
            Ok(result.race_flag.to_string().into())
        } else {
            Ok("".into())
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct RaceResultImportRequest {
    pub filename: StackString,
}

#[async_trait]
impl HandleRequest<RaceResultImportRequest> for PgPool {
    type Result = Result<(), Error>;
    async fn handle(&self, req: RaceResultImportRequest) -> Self::Result {
        if let Some(summary) = GarminSummary::get_by_filename(self, req.filename.as_str()).await? {
            let begin_datetime = summary.begin_datetime;
            let mut result: RaceResults = summary.into();
            if let Some(activity) =
                StravaActivity::get_by_begin_datetime(self, begin_datetime).await?
            {
                result.race_name = Some(activity.name);
            }
            result.race_filename = Some(req.filename);
            result.insert_into_db(self).await?;
        }
        Ok(())
    }
}
