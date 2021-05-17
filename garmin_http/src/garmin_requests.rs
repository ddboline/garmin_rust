use anyhow::format_err;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::future::try_join_all;
use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{collections::HashMap, path::PathBuf};
use tokio::{fs::remove_file, task::spawn_blocking};
use url::Url;

use fitbit_lib::{
    fitbit_client::{FitbitBodyWeightFatUpdateOutput, FitbitClient, FitbitUserProfile},
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_connect_lib::{
    garmin_connect_client::GarminConnectUserDailySummary,
    garmin_connect_hr_data::GarminConnectHrData,
};
use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_config::GarminConfig,
        garmin_connect_activity::GarminConnectActivity,
        garmin_correction_lap::GarminCorrectionLap,
        garmin_summary::{get_filename_from_datetime, get_list_of_files_from_db, GarminSummary},
        pgpool::PgPool,
        strava_activity::StravaActivity,
    },
    utils::sport_types::SportTypes,
    utils::{datetime_wrapper::DateTimeWrapper, naivedate_wrapper::NaiveDateWrapper},
};
use garmin_reports::garmin_constraints::GarminConstraints;
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::{StravaAthlete, StravaClient};

use crate::{errors::ServiceError as Error, garmin_rust_app::ConnectProxy};

pub struct GarminHtmlRequest {
    pub request: GarminRequest,
    pub is_demo: bool,
}

impl GarminHtmlRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        let body = GarminCli::from_pool(&pool)?
            .run_html(&self.request, self.is_demo)
            .await?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.request.constraints.to_query_string(), &pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Default)]
pub struct GarminListRequest {
    pub constraints: GarminConstraints,
}

impl From<GarminHtmlRequest> for GarminListRequest {
    fn from(item: GarminHtmlRequest) -> Self {
        Self {
            constraints: item.request.constraints,
        }
    }
}

impl GarminListRequest {
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.constraints.to_query_string(), &pool)
            .await
            .map_err(Into::into)
    }
}

impl GarminListRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        self.get_list_of_files_from_db(pool).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminUploadRequest {
    pub filename: PathBuf,
}

impl GarminUploadRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<DateTime<Utc>>, Error> {
        let gcli = GarminCli::from_pool(&pool)?;
        let filenames = vec![self.filename];
        let datetimes = gcli.process_filenames(&filenames).await?;
        gcli.sync_everything(false).await?;
        gcli.proc_everything().await?;
        Ok(datetimes)
    }
}

pub struct GarminConnectSyncRequest {}

impl GarminConnectSyncRequest {
    pub async fn handle(&self, pool: &PgPool, proxy: &ConnectProxy) -> Result<Vec<PathBuf>, Error> {
        let gcli = GarminCli::from_pool(pool)?;

        let max_timestamp = Utc::now() - Duration::days(30);

        let mut session = proxy.lock().await;
        session.init().await?;

        let new_activities = session.get_activities(max_timestamp).await?;

        let filenames = session
            .get_and_merge_activity_files(new_activities, pool)
            .await?;
        if !filenames.is_empty() {
            gcli.process_filenames(&filenames).await?;
            gcli.sync_everything(false).await?;
            gcli.proc_everything().await?;
        }
        GarminConnectActivity::fix_summary_id_in_db(pool).await?;
        Ok(filenames)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrSyncRequest {
    pub date: NaiveDateWrapper,
}

impl GarminConnectHrSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        proxy: &ConnectProxy,
        config: &GarminConfig,
    ) -> Result<GarminConnectHrData, Error> {
        let mut session = proxy.lock().await;
        session.init().await?;

        let heartrate_data = session.get_heartrate(self.date.into()).await?;
        FitbitClient::import_garmin_connect_heartrate(config.clone(), &heartrate_data).await?;
        let config = config.clone();
        FitbitHeartRate::calculate_summary_statistics(&config, pool, self.date.into()).await?;
        Ok(heartrate_data)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrApiRequest {
    pub date: NaiveDateWrapper,
}

impl GarminConnectHrApiRequest {
    pub async fn handle(&self, proxy: ConnectProxy) -> Result<Vec<FitbitHeartRate>, Error> {
        let mut session = proxy.lock().await;
        session.init().await?;

        let heartrate_data = session.get_heartrate(self.date.into()).await?;
        let hr_vals = FitbitHeartRate::from_garmin_connect_hr(&heartrate_data);
        Ok(hr_vals)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct StravaSyncRequest {
    pub start_datetime: Option<DateTimeWrapper>,
    pub end_datetime: Option<DateTimeWrapper>,
}

impl StravaSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<PathBuf>, Error> {
        let gcli = GarminCli::from_pool(&pool)?;

        let start_datetime = self
            .start_datetime
            .map(Into::into)
            .or_else(|| Some(Utc::now() - Duration::days(15)));
        let end_datetime = self
            .end_datetime
            .map(Into::into)
            .or_else(|| Some(Utc::now()));

        let client = StravaClient::with_auth(config.clone()).await?;
        let filenames = client
            .sync_with_client(start_datetime, end_datetime, pool)
            .await?;

        if !filenames.is_empty() {
            gcli.process_filenames(&filenames).await?;
            gcli.sync_everything(false).await?;
            gcli.proc_everything().await?;
        }
        StravaActivity::fix_summary_id_in_db(&pool).await?;

        Ok(filenames)
    }
}

pub struct GarminSyncRequest {}

impl GarminSyncRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let gcli = GarminCli::from_pool(pool)?;
        let mut output = gcli.sync_everything(false).await?;
        output.extend_from_slice(&gcli.proc_everything().await?);
        Ok(output)
    }
}

pub struct FitbitAuthRequest {}

impl FitbitAuthRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let client = FitbitClient::from_file(config.clone()).await?;
        let url = client.get_fitbit_auth_url().await?;
        Ok(url.as_str().into())
    }
}

pub struct FitbitRefreshRequest {}

impl FitbitRefreshRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let mut client = FitbitClient::from_file(config.clone()).await?;
        let body = client.refresh_fitbit_access_token().await?;
        client.to_file().await?;
        Ok(body)
    }
}

#[derive(Deserialize, Schema)]
pub struct FitbitCallbackRequest {
    code: StackString,
    state: StackString,
}

impl FitbitCallbackRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let mut client = FitbitClient::from_file(config.clone()).await?;
        let body = client
            .get_fitbit_access_token(&self.code, &self.state)
            .await?;
        client.to_file().await?;
        Ok(body)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitHeartrateApiRequest {
    date: NaiveDateWrapper,
}

impl FitbitHeartrateApiRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<FitbitHeartRate>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        client
            .get_fitbit_intraday_time_series_heartrate(self.date.into())
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitHeartrateCacheRequest {
    date: NaiveDateWrapper,
}

impl FitbitHeartrateCacheRequest {
    pub async fn handle(self, config: &GarminConfig) -> Result<Vec<FitbitHeartRate>, Error> {
        let config = config.clone();
        spawn_blocking(move || {
            FitbitHeartRate::read_avro_by_date(&config, self.date.into()).map_err(Into::into)
        })
        .await?
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitHeartrateUpdateRequest {
    updates: Vec<FitbitHeartRate>,
}

impl FitbitHeartrateUpdateRequest {
    pub async fn handle(self, config: &GarminConfig) -> Result<(), Error> {
        let config = config.clone();
        spawn_blocking(move || {
            FitbitHeartRate::merge_slice_to_avro(&config, &self.updates).map_err(Into::into)
        })
        .await?
    }
}

pub struct FitbitBodyWeightFatRequest {}

impl FitbitBodyWeightFatRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<FitbitBodyWeightFat>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        client.get_fitbit_bodyweightfat().await.map_err(Into::into)
    }
}

pub struct FitbitBodyWeightFatUpdateRequest {}

impl FitbitBodyWeightFatUpdateRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<FitbitBodyWeightFatUpdateOutput, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        client.sync_everything(pool).await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitSyncRequest {
    date: NaiveDateWrapper,
}

impl FitbitSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<FitbitHeartRate>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        let heartrates = client.import_fitbit_heartrate(self.date.into()).await?;
        FitbitHeartRate::calculate_summary_statistics(&client.config, &pool, self.date.into())
            .await?;
        Ok(heartrates)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitTcxSyncRequest {
    pub start_date: Option<NaiveDateWrapper>,
}

impl FitbitTcxSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<PathBuf>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        let start_date = self.start_date.map_or_else(
            || (Utc::now() - Duration::days(10)).naive_utc().date(),
            Into::into,
        );
        let filenames = client.sync_tcx(start_date).await?;

        let gcli = GarminCli::from_pool(pool)?;
        gcli.sync_everything(false).await?;
        gcli.proc_everything().await?;
        Ok(filenames)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Schema)]
pub struct ScaleMeasurementRequest {
    pub start_date: Option<NaiveDateWrapper>,
    pub end_date: Option<NaiveDateWrapper>,
    pub button_date: Option<NaiveDateWrapper>,
    pub offset: Option<usize>,
}

impl ScaleMeasurementRequest {
    fn add_default(&self, ndays: i64) -> Self {
        Self {
            start_date: match self.start_date {
                Some(d) => Some(d),
                None => Some(
                    (Local::now() - Duration::days(ndays))
                        .naive_utc()
                        .date()
                        .into(),
                ),
            },
            end_date: match self.end_date {
                Some(d) => Some(d),
                None => Some(Local::now().naive_utc().date().into()),
            },
            button_date: match self.button_date {
                Some(d) => Some(d),
                None => Some(Local::now().naive_utc().date().into()),
            },
            offset: self.offset,
        }
    }
}

impl ScaleMeasurementRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ScaleMeasurement>, Error> {
        ScaleMeasurement::read_from_db(
            pool,
            self.start_date.map(Into::into),
            self.end_date.map(Into::into),
        )
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

impl FitbitStatisticsPlotRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<HashMap<StackString, StackString>, Error> {
        let stats = FitbitStatisticsSummary::read_from_db(
            self.request.start_date.map(Into::into),
            self.request.end_date.map(Into::into),
            pool,
        )
        .await?;
        FitbitStatisticsSummary::get_fitbit_statistics_plots(&stats, self.request.offset)
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

impl ScaleMeasurementPlotRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<HashMap<StackString, StackString>, Error> {
        let measurements = ScaleMeasurement::read_from_db(
            pool,
            self.request.start_date.map(Into::into),
            self.request.end_date.map(Into::into),
        )
        .await?;
        ScaleMeasurement::get_scale_measurement_plots(&measurements, self.request.offset)
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
            start_date: item.start_date.expect("this should be impossible").into(),
            end_date: item.end_date.expect("this should be impossible").into(),
            button_date: item.button_date.map(Into::into),
            is_demo: false,
        }
    }
}

impl FitbitHeartratePlotRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<HashMap<StackString, StackString>, Error> {
        FitbitHeartRate::get_heartrate_plot(
            config,
            pool,
            self.start_date,
            self.end_date,
            self.button_date,
            self.is_demo,
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct ScaleMeasurementUpdateRequest {
    pub measurements: Vec<ScaleMeasurement>,
}

impl ScaleMeasurementUpdateRequest {
    pub async fn handle(&mut self, pool: &PgPool) -> Result<(), Error> {
        ScaleMeasurement::merge_updates(&mut self.measurements, pool).await?;
        Ok(())
    }
}

pub struct StravaAuthRequest {}

impl StravaAuthRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let client = StravaClient::from_file(config.clone()).await?;
        client
            .get_authorization_url_api()
            .await
            .map_err(Into::into)
            .map(|u| u.as_str().into())
    }
}

pub struct StravaRefreshRequest {}

impl StravaRefreshRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let mut client = StravaClient::from_file(config.clone()).await?;
        client.refresh_access_token().await?;
        client.to_file().await?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.into())
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

impl StravaCallbackRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let mut client = StravaClient::from_file(config.clone()).await?;
        client.process_callback(&self.code, &self.state).await?;
        client.to_file().await?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.into())
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaActivitiesRequest {
    pub start_date: Option<NaiveDateWrapper>,
    pub end_date: Option<NaiveDateWrapper>,
}

impl StravaActivitiesRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<StravaActivity>, Error> {
        let client = StravaClient::with_auth(config.clone()).await?;
        let start_date = self.start_date.map(|s| {
            DateTime::from_utc(
                NaiveDateTime::new(s.into(), NaiveTime::from_hms(0, 0, 0)),
                Utc,
            )
        });
        let end_date = self.end_date.map(|s| {
            DateTime::from_utc(
                NaiveDateTime::new(s.into(), NaiveTime::from_hms(23, 59, 59)),
                Utc,
            )
        });
        client
            .get_all_strava_activites(start_date, end_date)
            .await
            .map_err(Into::into)
    }
}

pub struct StravaActivitiesDBRequest(pub StravaActivitiesRequest);

impl StravaActivitiesDBRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StravaActivity>, Error> {
        StravaActivity::read_from_db(
            pool,
            self.0.start_date.map(Into::into),
            self.0.end_date.map(Into::into),
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaActiviesDBUpdateRequest {
    pub updates: Vec<StravaActivity>,
}

impl StravaActiviesDBUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let output = StravaActivity::upsert_activities(&self.updates, pool).await?;
        StravaActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaUploadRequest {
    pub filename: StackString,
    pub title: StackString,
    pub activity_type: StackString,
    pub description: Option<StackString>,
    pub is_private: Option<bool>,
}

impl StravaUploadRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let filename = config.gps_dir.join(self.filename.as_str());
        if !filename.exists() {
            return Ok(format!("File {} does not exist", self.filename).into());
        }
        let config = config.clone();
        let client = StravaClient::with_auth(config).await?;
        client
            .upload_strava_activity(
                &filename,
                &self.title,
                self.description.as_ref().map_or("", StackString::as_str),
            )
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaUpdateRequest {
    pub activity_id: u64,
    pub title: StackString,
    pub activity_type: StackString,
    pub description: Option<StackString>,
    pub is_private: Option<bool>,
    pub start_time: Option<DateTimeWrapper>,
}

impl StravaUpdateRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<Url, Error> {
        let sport = self.activity_type.parse()?;

        let config = config.clone();
        let client = StravaClient::with_auth(config).await?;
        let body = client
            .update_strava_activity(
                self.activity_id,
                &self.title,
                self.description.as_ref().map(StackString::as_str),
                sport,
                self.start_time.map(Into::into),
            )
            .await?;
        Ok(body)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaCreateRequest {
    pub filename: StackString,
}

impl StravaCreateRequest {
    pub async fn handle(&self, pool: &PgPool, config: &GarminConfig) -> Result<Option<i64>, Error> {
        if let Some(gfile) = GarminSummary::get_by_filename(pool, self.filename.as_str()).await? {
            let mut strava_activity: StravaActivity = gfile.into();
            let config = config.clone();
            let client = StravaClient::with_auth(config).await?;
            let activity_id = client.create_strava_activity(&strava_activity).await?;
            strava_activity.id = activity_id;
            strava_activity.insert_into_db(pool).await?;
            StravaActivity::fix_summary_id_in_db(pool).await?;
            Ok(Some(activity_id))
        } else {
            Ok(None)
        }
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct AddGarminCorrectionRequest {
    pub start_time: DateTimeWrapper,
    pub lap_number: i32,
    pub distance: Option<f64>,
    pub duration: Option<f64>,
    pub sport: Option<SportTypes>,
}

impl AddGarminCorrectionRequest {
    pub async fn handle(&self, pool: &PgPool, config: &GarminConfig) -> Result<StackString, Error> {
        let mut corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;
        let filename = get_filename_from_datetime(pool, self.start_time.into())
            .await?
            .ok_or_else(|| {
                format_err!(
                    "start_time {} doesn't match any existing file",
                    self.start_time
                )
            })?;
        let unique_key = (self.start_time.into(), self.lap_number);

        let mut new_corr = corr_map.get(&unique_key).map_or_else(
            || {
                GarminCorrectionLap::new()
                    .with_start_time(self.start_time.into())
                    .with_lap_number(self.lap_number)
            },
            |corr| *corr,
        );

        if self.distance.is_some() {
            new_corr.distance = self.distance;
        }
        if self.duration.is_some() {
            new_corr.duration = self.duration;
        }
        if self.sport.is_some() {
            new_corr.sport = self.sport;
        }

        corr_map.insert(unique_key, new_corr);

        GarminCorrectionLap::dump_corrections_to_db(&corr_map, pool).await?;
        GarminCorrectionLap::fix_corrections_in_db(pool).await?;

        let cache_path = config.cache_dir.join(&format!("{}.avro", filename));
        let summary_path = config
            .summary_cache
            .join(&format!("{}.summary.avro", filename));
        remove_file(cache_path).await?;
        remove_file(summary_path).await?;

        let gcli = GarminCli::from_pool(&pool)?;
        gcli.proc_everything().await?;

        Ok("".into())
    }
}

pub struct FitbitActivityTypesRequest {}

impl FitbitActivityTypesRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<HashMap<u64, StackString>, Error> {
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_fitbit_activity_types().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitActivitiesRequest {
    pub start_date: Option<NaiveDateWrapper>,
}

impl FitbitActivitiesRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<FitbitActivity>, Error> {
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        let start_date = self.start_date.map_or_else(
            || (Utc::now() - Duration::days(14)).naive_local().date(),
            Into::into,
        );
        client
            .get_all_activities(start_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectActivitiesRequest {
    pub start_date: Option<NaiveDateWrapper>,
}

impl GarminConnectActivitiesRequest {
    pub async fn handle(&self, proxy: &ConnectProxy) -> Result<Vec<GarminConnectActivity>, Error> {
        let start_date = self.start_date.map_or_else(
            || (Utc::now() - Duration::days(14)).naive_local().date(),
            Into::into,
        );
        let start_datetime = DateTime::from_utc(
            NaiveDateTime::new(start_date, NaiveTime::from_hms(0, 0, 0)),
            Utc,
        );
        let mut session = proxy.lock().await;
        session.init().await?;

        session
            .get_activities(start_datetime)
            .await
            .map_err(Into::into)
    }
}

pub struct StravaAthleteRequest {}

impl StravaAthleteRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<StravaAthlete, Error> {
        let config = config.clone();
        let client = StravaClient::with_auth(config).await?;
        client.get_strava_athlete().await.map_err(Into::into)
    }
}

pub struct FitbitProfileRequest {}

impl FitbitProfileRequest {
    pub async fn handle(&self, config: &GarminConfig) -> Result<FitbitUserProfile, Error> {
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_user_profile().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectUserSummaryRequest {
    pub date: Option<NaiveDateWrapper>,
}

impl GarminConnectUserSummaryRequest {
    pub async fn handle(
        &self,
        proxy: &ConnectProxy,
    ) -> Result<GarminConnectUserDailySummary, Error> {
        let mut session = proxy.lock().await;
        session.init().await?;

        let date = self
            .date
            .map_or_else(|| Local::now().naive_local().date(), Into::into);
        session.get_user_summary(date).await.map_err(Into::into)
    }
}

pub struct GarminConnectActivitiesDBRequest(pub StravaActivitiesRequest);

impl GarminConnectActivitiesDBRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<GarminConnectActivity>, Error> {
        GarminConnectActivity::read_from_db(
            pool,
            self.0.start_date.map(Into::into),
            self.0.end_date.map(Into::into),
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct GarminConnectActivitiesDBUpdateRequest {
    pub updates: Vec<GarminConnectActivity>,
}

impl GarminConnectActivitiesDBUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let output = GarminConnectActivity::upsert_activities(&self.updates, pool).await?;
        GarminConnectActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

pub struct FitbitActivitiesDBRequest(pub StravaActivitiesRequest);

impl FitbitActivitiesDBRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<FitbitActivity>, Error> {
        FitbitActivity::read_from_db(
            pool,
            self.0.start_date.map(Into::into),
            self.0.end_date.map(Into::into),
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct FitbitActivitiesDBUpdateRequest {
    pub updates: Vec<FitbitActivity>,
}

impl FitbitActivitiesDBUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let output = FitbitActivity::upsert_activities(&self.updates, pool).await?;
        FitbitActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

pub struct HeartrateStatisticsSummaryDBRequest(pub StravaActivitiesRequest);

impl HeartrateStatisticsSummaryDBRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<FitbitStatisticsSummary>, Error> {
        FitbitStatisticsSummary::read_from_db(
            self.0.start_date.map(Into::into),
            self.0.end_date.map(Into::into),
            pool,
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct HeartrateStatisticsSummaryDBUpdateRequest {
    pub updates: Vec<FitbitStatisticsSummary>,
}

impl HeartrateStatisticsSummaryDBUpdateRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let futures = self.updates.into_iter().map(|entry| {
            let pool = pool.clone();
            async move {
                entry.upsert_entry(&pool).await?;
                Ok(entry.date)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let mut output = vec!["update:".into()];
        output.extend(results?.into_iter().map(|d| d.to_string().into()));
        Ok(output)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultPlotRequest {
    pub race_type: RaceType,
    pub demo: Option<bool>,
}

impl RaceResultPlotRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<HashMap<StackString, StackString>, Error> {
        let model = RaceResultAnalysis::run_analysis(self.race_type, pool).await?;
        let demo = self.demo.unwrap_or(true);
        model.create_plot(demo).map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultFlagRequest {
    pub id: i32,
}

impl RaceResultFlagRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        if let Some(mut result) = RaceResults::get_result_by_id(self.id, pool).await? {
            result.race_flag = !result.race_flag;
            result.update_db(pool).await?;
            Ok(result.race_flag.to_string().into())
        } else {
            Ok("".into())
        }
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultImportRequest {
    pub filename: StackString,
}

impl RaceResultImportRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        if let Some(summary) = GarminSummary::get_by_filename(pool, self.filename.as_str()).await? {
            let begin_datetime = summary.begin_datetime;
            let mut result: RaceResults = summary.into();
            if let Some(activity) =
                StravaActivity::get_by_begin_datetime(pool, begin_datetime).await?
            {
                result.race_name = Some(activity.name);
            }
            result.insert_into_db(pool).await?;
            result.set_race_id(pool).await?;
            result.update_race_summary_ids(pool).await?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultsDBRequest {
    pub race_type: Option<RaceType>,
}

impl RaceResultsDBRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<RaceResults>, Error> {
        let race_type = self.race_type.unwrap_or(RaceType::Personal);
        RaceResults::get_results_by_type(race_type, pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultsDBUpdateRequest {
    pub updates: Vec<RaceResults>,
}

impl RaceResultsDBUpdateRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<(), Error> {
        let futures = self.updates.into_iter().map(|mut result| {
            let pool = pool.clone();
            async move { result.upsert_db(&pool).await.map_err(Into::into) }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        Ok(())
    }
}
