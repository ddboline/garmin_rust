use futures::future::try_join_all;
use rweb::Schema;
use rweb_helper::{DateTimeType, DateType};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::collections::BTreeSet;
use time::{macros::time, Date, Duration, OffsetDateTime};
use time_tz::OffsetDateTimeExt;
use tokio::task::spawn_blocking;
use url::Url;

use fitbit_lib::{
    fitbit_heartrate::FitbitHeartRate, fitbit_statistics_summary::FitbitStatisticsSummary,
};
use garmin_cli::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::{date_time_wrapper::DateTimeWrapper, garmin_config::GarminConfig};
use garmin_models::{
    garmin_correction_lap::GarminCorrectionLap, garmin_summary::GarminSummary,
    strava_activity::StravaActivity,
};
use garmin_reports::garmin_constraints::GarminConstraints;
use garmin_utils::pgpool::PgPool;
use strava_lib::strava_client::StravaClient;

use crate::{
    errors::ServiceError as Error, sport_types_wrapper::SportTypesWrapper, FitbitHeartRateWrapper,
    FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper, ScaleMeasurementWrapper,
};

pub struct GarminHtmlRequest {
    pub request: GarminRequest,
    pub is_demo: bool,
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

#[derive(Serialize, Deserialize, Schema)]
pub struct StravaSyncRequest {
    pub start_datetime: Option<DateTimeType>,
    pub end_datetime: Option<DateTimeType>,
}

impl StravaSyncRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn run_sync(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Vec<StravaActivity>, Error> {
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
        let activities = client
            .sync_with_client(start_datetime, end_datetime, pool)
            .await?;

        if !activities.is_empty() {
            gcli.sync_everything().await?;
            gcli.proc_everything().await?;
        }
        StravaActivity::fix_summary_id_in_db(pool).await?;

        Ok(activities)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitHeartrateCacheRequest {
    date: DateType,
}

impl FitbitHeartrateCacheRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn get_cache(self, config: &GarminConfig) -> Result<Vec<FitbitHeartRate>, Error> {
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
    pub async fn merge_data(self, config: &GarminConfig) -> Result<BTreeSet<Date>, Error> {
        let config = config.clone();
        let mut updates: Vec<_> = self.updates.into_iter().map(Into::into).collect();
        updates.shrink_to_fit();
        spawn_blocking(move || {
            FitbitHeartRate::merge_slice_to_avro(&config, &updates).map_err(Into::into)
        })
        .await?
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitTcxSyncRequest {
    pub start_date: Option<DateType>,
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
    #[schema(description = "Limit")]
    pub limit: Option<usize>,
}

impl ScaleMeasurementRequest {
    fn add_default(&self, ndays: i64) -> Self {
        let local = DateTimeWrapper::local_tz();
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
                None => Some(OffsetDateTime::now_utc().date().into()),
            },
            button_date: match self.button_date {
                Some(d) => Some(d),
                None => Some(OffsetDateTime::now_utc().date().into()),
            },
            offset: self.offset,
            limit: self.limit,
        }
    }
}

pub struct FitbitStatisticsPlotRequest {
    pub start_date: DateType,
    pub end_date: DateType,
    pub offset: usize,
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for FitbitStatisticsPlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(365);
        Self {
            start_date: item.start_date.expect("this should be impossible"),
            end_date: item.end_date.expect("this should be impossible"),
            offset: item.offset.unwrap_or(0),
            is_demo: false,
        }
    }
}

pub struct ScaleMeasurementPlotRequest {
    pub start_date: DateType,
    pub end_date: DateType,
    pub offset: usize,
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for ScaleMeasurementPlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(365);
        Self {
            start_date: item.start_date.expect("this should be impossible"),
            end_date: item.end_date.expect("this should be impossible"),
            offset: item.offset.unwrap_or(0),
            is_demo: false,
        }
    }
}

#[derive(Clone, Copy)]
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

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct ScaleMeasurementUpdateRequest {
    pub measurements: Vec<ScaleMeasurementWrapper>,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StravaActivitiesRequest {
    #[schema(description = "Start Date")]
    pub start_date: Option<DateType>,
    #[schema(description = "End Date")]
    pub end_date: Option<DateType>,
    #[schema(description = "Offset")]
    pub offset: Option<usize>,
    #[schema(description = "Limit")]
    pub limit: Option<usize>,
}

impl StravaActivitiesRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn get_activities(
        &self,
        config: &GarminConfig,
    ) -> Result<Vec<StravaActivity>, Error> {
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
    pub async fn run_upload(&self, config: &GarminConfig) -> Result<StackString, Error> {
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
    pub async fn run_update(&self, config: &GarminConfig) -> Result<Url, Error> {
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
    pub async fn create_activity(
        &self,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Result<Option<i64>, Error> {
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
    pub async fn add_corrections(self, pool: &PgPool) -> Result<StackString, Error> {
        let mut corr_map = GarminCorrectionLap::read_corrections_from_db(pool).await?;
        corr_map.shrink_to_fit();
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

#[derive(Serialize, Deserialize, Schema)]
pub struct FitbitActivitiesRequest {
    pub start_date: Option<DateType>,
}

#[derive(Serialize, Deserialize, Schema)]
pub struct GarminConnectActivitiesRequest {
    pub start_date: Option<DateType>,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct GarminConnectActivitiesDBUpdateRequest {
    pub updates: Vec<GarminConnectActivityWrapper>,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct HeartrateStatisticsSummaryDBUpdateRequest {
    pub updates: Vec<FitbitStatisticsSummaryWrapper>,
}

impl HeartrateStatisticsSummaryDBUpdateRequest {
    /// # Errors
    /// Returns error if db query fails
    pub async fn process_updates(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
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
