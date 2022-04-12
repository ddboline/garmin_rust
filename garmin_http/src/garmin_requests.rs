use futures::future::try_join_all;
use rweb::Schema;
use rweb_helper::{DateTimeType, DateType};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, path::PathBuf};
use time::{macros::time, Date, Duration, OffsetDateTime};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};
use tokio::task::spawn_blocking;
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
        garmin_summary::{get_list_of_files_from_db, GarminSummary},
        pgpool::PgPool,
        strava_activity::StravaActivity,
    },
    utils::date_time_wrapper::DateTimeWrapper,
};
use garmin_reports::garmin_constraints::GarminConstraints;
use race_result_analysis::{
    race_result_analysis::RaceResultAnalysis, race_results::RaceResults, race_type::RaceType,
};
use strava_lib::strava_client::{StravaAthlete, StravaClient};

use crate::{
    errors::ServiceError as Error, garmin_rust_app::ConnectProxy,
    sport_types_wrapper::SportTypesWrapper, FitbitActivityWrapper, FitbitHeartRateWrapper,
    FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper, RaceResultsWrapper,
    RaceTypeWrapper, ScaleMeasurementWrapper, StravaActivityWrapper,
};

pub struct GarminHtmlRequest {
    pub request: GarminRequest,
    pub is_demo: bool,
}

impl GarminHtmlRequest {
    /// # Errors
    /// Returns error if config init fails
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        let body = GarminCli::from_pool(pool)?
            .run_html(&self.request, self.is_demo)
            .await?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.request.constraints.to_query_string(), pool)
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
    /// # Errors
    /// Returns error if db query fails
    pub async fn get_list_of_files_from_db(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        get_list_of_files_from_db(&self.constraints.to_query_string(), pool)
            .await
            .map_err(Into::into)
    }
}

impl GarminListRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        self.get_list_of_files_from_db(pool).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminUploadRequest {
    pub filename: PathBuf,
}

impl GarminUploadRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<DateTimeWrapper>, Error> {
        let gcli = GarminCli::from_pool(pool)?;
        let filenames = vec![self.filename];
        let datetimes = gcli.process_filenames(&filenames).await?;
        gcli.sync_everything(false).await?;
        gcli.proc_everything().await?;
        Ok(datetimes)
    }
}

pub struct GarminConnectSyncRequest {}

impl GarminConnectSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool, proxy: &ConnectProxy) -> Result<Vec<PathBuf>, Error> {
        let gcli = GarminCli::from_pool(pool)?;

        let max_timestamp = OffsetDateTime::now_utc() - Duration::days(30);

        let mut session = proxy.lock().await;
        session.init().await?;

        let new_activities = session.get_activities(Some(max_timestamp)).await?;

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
    pub date: DateType,
}

impl GarminConnectHrSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        pool: &PgPool,
        proxy: &ConnectProxy,
        config: &GarminConfig,
    ) -> Result<GarminConnectHrData, Error> {
        let mut session = proxy.lock().await;
        session.init().await?;
        let date = self.date.into();
        let heartrate_data = session.get_heartrate(date).await?;
        FitbitClient::import_garmin_connect_heartrate(config.clone(), &heartrate_data).await?;
        let config = config.clone();
        FitbitHeartRate::calculate_summary_statistics(&config, pool, date).await?;
        Ok(heartrate_data)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectHrApiRequest {
    pub date: DateType,
}

impl GarminConnectHrApiRequest {
    /// # Errors
    /// Returns error if db query fails
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
    pub start_datetime: Option<DateTimeType>,
    pub end_datetime: Option<DateTimeType>,
}

impl StravaSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<PathBuf>, Error> {
        let gcli = GarminCli::from_pool(pool)?;

        let start_datetime = self
            .start_datetime
            .map(Into::into)
            .or_else(|| Some(OffsetDateTime::now_utc() - Duration::days(15)));
        let end_datetime = self
            .end_datetime
            .map(Into::into)
            .or_else(|| Some(OffsetDateTime::now_utc()));

        let client = StravaClient::with_auth(config.clone()).await?;
        let filenames = client
            .sync_with_client(start_datetime, end_datetime, pool)
            .await?;

        if !filenames.is_empty() {
            gcli.process_filenames(&filenames).await?;
            gcli.sync_everything(false).await?;
            gcli.proc_everything().await?;
        }
        StravaActivity::fix_summary_id_in_db(pool).await?;

        Ok(filenames)
    }
}

pub struct GarminSyncRequest {}

impl GarminSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let gcli = GarminCli::from_pool(pool)?;
        let mut output = gcli.sync_everything(false).await?;
        output.extend_from_slice(&gcli.proc_everything().await?);
        Ok(output)
    }
}

pub struct FitbitAuthRequest {}

impl FitbitAuthRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let client = FitbitClient::from_file(config.clone()).await?;
        let url = client.get_fitbit_auth_url().await?;
        Ok(url.as_str().into())
    }
}

pub struct FitbitRefreshRequest {}

impl FitbitRefreshRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let mut client = FitbitClient::from_file(config.clone()).await?;
        let body = client.refresh_fitbit_access_token().await?;
        client.to_file().await?;
        Ok(body)
    }
}

#[derive(Deserialize, Schema)]
pub struct FitbitCallbackRequest {
    #[schema(description = "Authorization Code")]
    code: StackString,
    #[schema(description = "CSRF State")]
    state: StackString,
}

impl FitbitCallbackRequest {
    /// # Errors
    /// Returns error if db query fails
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
    date: DateType,
}

impl FitbitHeartrateApiRequest {
    /// # Errors
    /// Returns error if db query fails
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
    date: DateType,
}

impl FitbitHeartrateCacheRequest {
    /// # Errors
    /// Returns error if db query fails
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
    updates: Vec<FitbitHeartRateWrapper>,
}

impl FitbitHeartrateUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, config: &GarminConfig) -> Result<(), Error> {
        let config = config.clone();
        let updates: Vec<_> = self.updates.into_iter().map(Into::into).collect();
        spawn_blocking(move || {
            FitbitHeartRate::merge_slice_to_avro(&config, &updates).map_err(Into::into)
        })
        .await?
    }
}

pub struct FitbitBodyWeightFatRequest {}

impl FitbitBodyWeightFatRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<FitbitBodyWeightFat>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        client.get_fitbit_bodyweightfat().await.map_err(Into::into)
    }
}

pub struct FitbitBodyWeightFatUpdateRequest {}

impl FitbitBodyWeightFatUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
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
    date: DateType,
}

impl FitbitSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<FitbitHeartRate>, Error> {
        let date = self.date.into();
        let client = FitbitClient::with_auth(config.clone()).await?;
        let heartrates = client.import_fitbit_heartrate(date).await?;
        FitbitHeartRate::calculate_summary_statistics(&client.config, pool, date).await?;
        Ok(heartrates)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitTcxSyncRequest {
    pub start_date: Option<DateType>,
}

impl FitbitTcxSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<PathBuf>, Error> {
        let client = FitbitClient::with_auth(config.clone()).await?;
        let start_date = self.start_date.map_or_else(
            || (OffsetDateTime::now_utc() - Duration::days(10)).date(),
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
    #[schema(description = "Start Date")]
    pub start_date: Option<DateType>,
    #[schema(description = "End Date")]
    pub end_date: Option<DateType>,
    #[schema(description = "Button Date")]
    pub button_date: Option<DateType>,
    #[schema(description = "Offset")]
    pub offset: Option<usize>,
}

impl ScaleMeasurementRequest {
    fn add_default(&self, ndays: i64) -> Self {
        let local = time_tz::system::get_timezone().unwrap_or(UTC);
        Self {
            start_date: match self.start_date {
                Some(d) => Some(d),
                None => Some(
                    (OffsetDateTime::now_utc() - Duration::days(ndays))
                        .to_timezone(local)
                        .date()
                        .into(),
                ),
            },
            end_date: match self.end_date {
                Some(d) => Some(d),
                None => Some(OffsetDateTime::now_utc().to_timezone(local).date().into()),
            },
            button_date: match self.button_date {
                Some(d) => Some(d),
                None => Some(OffsetDateTime::now_utc().to_timezone(local).date().into()),
            },
            offset: self.offset,
        }
    }
}

impl ScaleMeasurementRequest {
    /// # Errors
    /// Returns error if db query fails
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
    /// # Errors
    /// Returns error if db query fails
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
    /// # Errors
    /// Returns error if db query fails
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
    pub start_date: DateType,
    pub end_date: DateType,
    pub button_date: Option<DateType>,
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

impl FitbitHeartratePlotRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<HashMap<StackString, StackString>, Error> {
        FitbitHeartRate::get_heartrate_plot(
            config,
            pool,
            self.start_date.into(),
            self.end_date.into(),
            self.button_date.map(Into::into),
            self.is_demo,
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct ScaleMeasurementUpdateRequest {
    pub measurements: Vec<ScaleMeasurementWrapper>,
}

impl ScaleMeasurementUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<(), Error> {
        let mut measurements: Vec<_> = self.measurements.into_iter().map(Into::into).collect();
        ScaleMeasurement::merge_updates(&mut measurements, pool).await?;
        Ok(())
    }
}

pub struct StravaAuthRequest {}

impl StravaAuthRequest {
    /// # Errors
    /// Returns error if db query fails
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
    /// # Errors
    /// Returns error if db query fails
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
    #[schema(description = "Authorization Code")]
    pub code: StackString,
    #[schema(description = "CSRF State")]
    pub state: StackString,
}

impl StravaCallbackRequest {
    /// # Errors
    /// Returns error if db query fails
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
    #[schema(description = "Start Date")]
    pub start_date: Option<DateType>,
    #[schema(description = "End Date")]
    pub end_date: Option<DateType>,
}

impl StravaActivitiesRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<StravaActivity>, Error> {
        let client = StravaClient::with_auth(config.clone()).await?;
        let start_date = self.start_date.map(|s| {
            let d: Date = s.into();
            d.with_time(time!(00:00:00)).assume_utc()
        });
        let end_date = self.end_date.map(|s| {
            let d: Date = s.into();
            d.with_time(time!(23:59:59)).assume_utc()
        });
        client
            .get_all_strava_activites(start_date, end_date)
            .await
            .map_err(Into::into)
    }
}

pub struct StravaActivitiesDBRequest(pub StravaActivitiesRequest);

impl StravaActivitiesDBRequest {
    /// # Errors
    /// Returns error if db query fails
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
    pub updates: Vec<StravaActivityWrapper>,
}

impl StravaActiviesDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let updates: Vec<_> = self.updates.into_iter().map(Into::into).collect();
        let output = StravaActivity::upsert_activities(&updates, pool).await?;
        StravaActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaUploadRequest {
    #[schema(description = "File Name")]
    pub filename: StackString,
    #[schema(description = "Title")]
    pub title: StackString,
    #[schema(description = "Activity Type")]
    pub activity_type: StackString,
    #[schema(description = "Description")]
    pub description: Option<StackString>,
    #[schema(description = "Privacy Flag")]
    pub is_private: Option<bool>,
}

impl StravaUploadRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<StackString, Error> {
        let filename = config.gps_dir.join(self.filename.as_str());
        if !filename.exists() {
            return Ok(format_sstr!("File {} does not exist", self.filename));
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
    #[schema(description = "Strava Activity ID")]
    pub activity_id: u64,
    #[schema(description = "Title")]
    pub title: StackString,
    #[schema(description = "Activity Type")]
    pub activity_type: StackString,
    #[schema(description = "Description")]
    pub description: Option<StackString>,
    #[schema(description = "Privacy Flag")]
    pub is_private: Option<bool>,
    #[schema(description = "Start DateTime")]
    pub start_time: Option<DateTimeType>,
}

impl StravaUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
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
    /// # Errors
    /// Returns error if db query fails
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
    #[schema(description = "Start DateTime")]
    pub start_time: DateTimeType,
    #[schema(description = "Lap Number")]
    pub lap_number: i32,
    #[schema(description = "Distance (m)")]
    pub distance: Option<f64>,
    #[schema(description = "Duration (s)")]
    pub duration: Option<f64>,
    #[schema(description = "Sport")]
    pub sport: Option<SportTypesWrapper>,
}

impl AddGarminCorrectionRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<StackString, Error> {
        let mut corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;
        let start_time: OffsetDateTime = self.start_time.into();
        let start_time = start_time.into();
        let unique_key = (start_time, self.lap_number);

        let mut new_corr = corr_map.get(&unique_key).map_or_else(
            || {
                GarminCorrectionLap::new()
                    .with_start_time(start_time)
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
            new_corr.sport = self.sport.map(Into::into);
        }

        corr_map.insert(unique_key, new_corr);

        GarminCorrectionLap::dump_corrections_to_db(&corr_map, pool).await?;
        GarminCorrectionLap::fix_corrections_in_db(pool).await?;

        let gcli = GarminCli::from_pool(pool)?;
        gcli.proc_everything().await?;

        Ok("".into())
    }
}

pub struct FitbitActivityTypesRequest {}

impl FitbitActivityTypesRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        config: &GarminConfig,
    ) -> Result<HashMap<StackString, StackString>, Error> {
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_fitbit_activity_types().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitActivitiesRequest {
    pub start_date: Option<DateType>,
}

impl FitbitActivitiesRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<Vec<FitbitActivity>, Error> {
        let local = time_tz::system::get_timezone().unwrap_or(UTC);
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        let start_date = self.start_date.map_or_else(
            || {
                (OffsetDateTime::now_utc() - Duration::days(14))
                    .to_timezone(local)
                    .date()
            },
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
    pub start_date: Option<DateType>,
}

impl GarminConnectActivitiesRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, proxy: &ConnectProxy) -> Result<Vec<GarminConnectActivity>, Error> {
        let local = time_tz::system::get_timezone().unwrap_or(UTC);
        let start_date = self.start_date.map_or_else(
            || {
                (OffsetDateTime::now_utc() - Duration::days(14))
                    .to_timezone(local)
                    .date()
            },
            Into::into,
        );
        let start_datetime = start_date.with_time(time!(00:00:00)).assume_utc();
        let mut session = proxy.lock().await;
        session.init().await?;

        session
            .get_activities(Some(start_datetime))
            .await
            .map_err(Into::into)
    }
}

pub struct StravaAthleteRequest {}

impl StravaAthleteRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<StravaAthlete, Error> {
        let config = config.clone();
        let client = StravaClient::with_auth(config).await?;
        client.get_strava_athlete().await.map_err(Into::into)
    }
}

pub struct FitbitProfileRequest {}

impl FitbitProfileRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, config: &GarminConfig) -> Result<FitbitUserProfile, Error> {
        let config = config.clone();
        let client = FitbitClient::with_auth(config).await?;
        client.get_user_profile().await.map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectUserSummaryRequest {
    pub date: Option<DateType>,
}

impl GarminConnectUserSummaryRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(
        &self,
        proxy: &ConnectProxy,
    ) -> Result<GarminConnectUserDailySummary, Error> {
        let local = time_tz::system::get_timezone().unwrap_or(UTC);
        let mut session = proxy.lock().await;
        session.init().await?;

        let date = self.date.map_or_else(
            || OffsetDateTime::now_utc().to_timezone(local).date(),
            Into::into,
        );
        session.get_user_summary(date).await.map_err(Into::into)
    }
}

pub struct GarminConnectActivitiesDBRequest(pub StravaActivitiesRequest);

impl GarminConnectActivitiesDBRequest {
    /// # Errors
    /// Returns error if db query fails
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
    pub updates: Vec<GarminConnectActivityWrapper>,
}

impl GarminConnectActivitiesDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let updates: Vec<_> = self.updates.into_iter().map(Into::into).collect();
        let output = GarminConnectActivity::upsert_activities(&updates, pool).await?;
        GarminConnectActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

pub struct FitbitActivitiesDBRequest(pub StravaActivitiesRequest);

impl FitbitActivitiesDBRequest {
    /// # Errors
    /// Returns error if db query fails
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
    pub updates: Vec<FitbitActivityWrapper>,
}

impl FitbitActivitiesDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let updates: Vec<_> = self.updates.into_iter().map(Into::into).collect();
        let output = FitbitActivity::upsert_activities(&updates, pool).await?;
        FitbitActivity::fix_summary_id_in_db(pool).await?;
        Ok(output)
    }
}

pub struct HeartrateStatisticsSummaryDBRequest(pub StravaActivitiesRequest);

impl HeartrateStatisticsSummaryDBRequest {
    /// # Errors
    /// Returns error if db query fails
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
    pub updates: Vec<FitbitStatisticsSummaryWrapper>,
}

impl HeartrateStatisticsSummaryDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        let futures = self.updates.into_iter().map(|entry| {
            let pool = pool.clone();
            let entry: FitbitStatisticsSummary = entry.into();
            async move {
                entry.upsert_entry(&pool).await?;
                let date_str = StackString::from_display(entry.date);
                Ok(date_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let mut output = vec!["update:".into()];
        output.extend_from_slice(&results?);
        Ok(output)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultPlotRequest {
    #[schema(description = "Race Type")]
    pub race_type: RaceTypeWrapper,
    #[schema(description = "Demo Flag")]
    pub demo: Option<bool>,
}

impl RaceResultPlotRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<HashMap<StackString, StackString>, Error> {
        let model = RaceResultAnalysis::run_analysis(self.race_type.into(), pool).await?;
        let demo = self.demo.unwrap_or(true);
        model.create_plot(demo).map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultFlagRequest {
    pub id: i32,
}

impl RaceResultFlagRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        if let Some(mut result) = RaceResults::get_result_by_id(self.id, pool).await? {
            result.race_flag = !result.race_flag;
            let flag_str = StackString::from_display(result.race_flag);
            result.update_db(pool).await?;
            Ok(flag_str)
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
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        if let Some(summary) = GarminSummary::get_by_filename(pool, self.filename.as_str()).await? {
            let begin_datetime = summary.begin_datetime.into();
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
    #[schema(description = "Race Type")]
    pub race_type: Option<RaceTypeWrapper>,
}

impl RaceResultsDBRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<RaceResults>, Error> {
        let race_type = self.race_type.map_or(RaceType::Personal, Into::into);
        RaceResults::get_results_by_type(race_type, pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct RaceResultsDBUpdateRequest {
    pub updates: Vec<RaceResultsWrapper>,
}

impl RaceResultsDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn handle(self, pool: &PgPool) -> Result<(), Error> {
        let futures = self.updates.into_iter().map(|result| {
            let pool = pool.clone();
            let mut result: RaceResults = result.into();
            async move { result.upsert_db(&pool).await.map_err(Into::into) }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        Ok(())
    }
}
