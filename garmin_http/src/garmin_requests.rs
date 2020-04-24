use anyhow::format_err;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::future::try_join_all;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::Path,
    sync::Arc,
};
use tokio::fs::remove_file;
use tokio::task::spawn_blocking;

use fitbit_lib::{
    fitbit_client::FitbitClient,
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    scale_measurement::ScaleMeasurement,
};

use strava_lib::strava_client::{StravaAuthType, StravaClient};

use garmin_lib::{
    common::{
        garmin_cli::{GarminCli, GarminRequest},
        garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionList},
        garmin_summary::get_list_of_files_from_db,
        pgpool::PgPool,
        strava_sync::{
            get_strava_id_maximum_begin_datetime, get_strava_ids, upsert_strava_id, StravaItem,
        },
    },
    utils::sport_types::SportTypes,
    utils::stack_string::StackString,
};

use crate::{errors::ServiceError as Error, CONFIG};

#[async_trait]
pub trait HandleRequest<T> {
    type Result;
    async fn handle(&self, req: T) -> Self::Result;
}

pub struct GarminCorrRequest {}

#[async_trait]
impl HandleRequest<GarminCorrRequest> for PgPool {
    type Result = Result<GarminCorrectionList, Error>;
    async fn handle(&self, _: GarminCorrRequest) -> Self::Result {
        GarminCorrectionList::new(&self)
            .read_corrections_from_db()
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
    type Result = Result<String, Error>;
    async fn handle(&self, msg: GarminHtmlRequest) -> Self::Result {
        let body = GarminCli::from_pool(&self)?
            .run_html(&msg.request, msg.is_demo)
            .await?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    pub async fn get_list_of_files_from_db(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
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
    pub async fn get_list_of_files_from_db(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
        get_list_of_files_from_db(&self.constraints.join(" OR "), &pool)
            .await
            .map_err(Into::into)
    }
}

#[async_trait]
impl HandleRequest<GarminListRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    async fn handle(&self, msg: GarminListRequest) -> Self::Result {
        msg.get_list_of_files_from_db(self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminUploadRequest {
    pub filename: StackString,
}

#[async_trait]
impl HandleRequest<GarminUploadRequest> for PgPool {
    type Result = Result<Vec<StackString>, Error>;
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
    type Result = Result<Vec<String>, Error>;
    async fn handle(&self, _: GarminConnectSyncRequest) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let filenames = gcli.sync_with_garmin_connect().await?;
        gcli.proc_everything().await?;
        Ok(filenames)
    }
}

pub struct StravaSyncRequest {}

#[async_trait]
impl HandleRequest<StravaSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    async fn handle(&self, _: StravaSyncRequest) -> Self::Result {
        let config = CONFIG.clone();

        let max_datetime = get_strava_id_maximum_begin_datetime(&self).await?;
        let max_datetime = match max_datetime {
            Some(dt) => {
                let max_datetime = dt - Duration::days(14);
                debug!("max_datetime {}", max_datetime);
                Some(max_datetime)
            }
            None => None,
        };
        let activities = spawn_blocking(move || {
            let client = StravaClient::from_file(config, Some(StravaAuthType::Read))?;
            client.get_strava_activites(max_datetime, None)
        })
        .await??;

        upsert_strava_id(&activities, &self)
            .await
            .map_err(Into::into)
    }
}

pub struct GarminSyncRequest {}

#[async_trait]
impl HandleRequest<GarminSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
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
    type Result = Result<String, Error>;
    async fn handle(&self, _: FitbitAuthRequest) -> Self::Result {
        let config = CONFIG.clone();
        spawn_blocking(move || {
            let client = FitbitClient::from_file(config)?;
            let url = client.get_fitbit_auth_url()?;
            Ok(url)
        })
        .await?
    }
}

#[derive(Deserialize)]
pub struct FitbitCallbackRequest {
    code: String,
}

#[async_trait]
impl HandleRequest<FitbitCallbackRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: FitbitCallbackRequest) -> Self::Result {
        let config = CONFIG.clone();
        spawn_blocking(move || {
            let mut client = FitbitClient::from_file(config)?;
            let url = client.get_fitbit_access_token(&msg.code)?;
            client.to_file()?;
            Ok(url)
        })
        .await?
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
        spawn_blocking(move || {
            let client = FitbitClient::from_file(config)?;
            client
                .get_fitbit_intraday_time_series_heartrate(msg.date)
                .map_err(Into::into)
        })
        .await?
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
        spawn_blocking(move || {
            let client = FitbitClient::from_file(config)?;
            client.get_fitbit_bodyweightfat().map_err(Into::into)
        })
        .await?
    }
}

pub struct FitbitBodyWeightFatUpdateRequest {}

#[async_trait]
impl HandleRequest<FitbitBodyWeightFatUpdateRequest> for PgPool {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
    async fn handle(&self, _: FitbitBodyWeightFatUpdateRequest) -> Self::Result {
        let start_date: NaiveDate = (Local::now() - Duration::days(30)).naive_local().date();
        let config = CONFIG.clone();
        let client = Arc::new(FitbitClient::from_file(config)?);
        let existing_map: Result<HashMap<NaiveDate, _>, Error> = {
            let client = client.clone();
            spawn_blocking(move || {
                let measurements: HashMap<_, _> = client
                    .get_fitbit_bodyweightfat()?
                    .into_iter()
                    .map(|entry| {
                        let date = entry.datetime.with_timezone(&Local).naive_local().date();
                        (date, entry)
                    })
                    .collect();
                Ok(measurements)
            })
            .await?
        };

        let existing_map = existing_map?;

        let new_measurements: Vec<_> = ScaleMeasurement::read_from_db(self, Some(start_date), None)
            .await?
            .into_iter()
            .filter(|entry| {
                let date = entry.datetime.with_timezone(&Local).naive_local().date();
                !existing_map.contains_key(&date)
            })
            .collect();
        spawn_blocking(move || {
            let new_measurements = client.update_fitbit_bodyweightfat(new_measurements)?;
            Ok(new_measurements)
        })
        .await?
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
        spawn_blocking(move || {
            let client = FitbitClient::from_file(config)?;
            client
                .import_fitbit_heartrate(msg.date, &client.config)
                .map_err(Into::into)
        })
        .await?
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitTcxSyncRequest {
    pub start_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<FitbitTcxSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    async fn handle(&self, msg: FitbitTcxSyncRequest) -> Self::Result {
        let config = CONFIG.clone();

        let results = spawn_blocking(move || {
            let client = FitbitClient::from_file(config.clone())?;
            let start_date = msg
                .start_date
                .unwrap_or_else(|| (Utc::now() - Duration::days(10)).naive_utc().date());
            let results: Result<Vec<_>, Error> = client
                .get_tcx_urls(start_date)?
                .into_iter()
                .filter_map(|(start_time, tcx_url)| {
                    let res = || {
                        let fname = format!(
                            "{}/{}.tcx",
                            config.gps_dir,
                            start_time.format("%Y-%m-%d_%H-%M-%S_1_1").to_string(),
                        );
                        if Path::new(&fname).exists() {
                            Ok(None)
                        } else {
                            client.download_tcx(&tcx_url, &mut File::create(&fname)?)?;
                            Ok(Some(fname))
                        }
                    };
                    res().transpose()
                })
                .collect();
            results
        })
        .await?;

        let gcli = GarminCli::from_pool(&self)?;
        gcli.proc_everything().await?;
        results
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScaleMeasurementRequest {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
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
    type Result = Result<String, Error>;
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
    pub is_demo: bool,
}

impl From<ScaleMeasurementRequest> for FitbitHeartratePlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(3);
        Self {
            start_date: item.start_date.expect("this should be impossible"),
            end_date: item.end_date.expect("this should be impossible"),
            is_demo: false,
        }
    }
}

#[async_trait]
impl HandleRequest<FitbitHeartratePlotRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, req: FitbitHeartratePlotRequest) -> Self::Result {
        let config = CONFIG.clone();
        FitbitHeartRate::get_heartrate_plot(
            &config,
            self,
            req.start_date,
            req.end_date,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaAuthRequest {
    pub auth_type: Option<String>,
}

#[async_trait]
impl HandleRequest<StravaAuthRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: StravaAuthRequest) -> Self::Result {
        let config = CONFIG.clone();
        let auth_type = msg.auth_type.and_then(|a| match a.as_str() {
            "read" => Some(StravaAuthType::Read),
            "write" => Some(StravaAuthType::Write),
            _ => None,
        });
        spawn_blocking(move || {
            let client = StravaClient::from_file(config, auth_type)?;
            client.get_authorization_url().map_err(Into::into)
        })
        .await?
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaCallbackRequest {
    pub code: String,
    pub state: String,
}

#[async_trait]
impl HandleRequest<StravaCallbackRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: StravaCallbackRequest) -> Self::Result {
        let config = CONFIG.clone();
        spawn_blocking(move || {
            let mut client = StravaClient::from_file(config, None)?;
            client.process_callback(&msg.code, &msg.state)?;
            client.to_file()?;
            let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
            Ok(body.into())
        })
        .await?
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaActivitiesRequest {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[async_trait]
impl HandleRequest<StravaActivitiesRequest> for PgPool {
    type Result = Result<HashMap<StackString, StravaItem>, Error>;
    async fn handle(&self, msg: StravaActivitiesRequest) -> Self::Result {
        let config = CONFIG.clone();
        spawn_blocking(move || {
            let client = StravaClient::from_file(config, Some(StravaAuthType::Read))?;
            let start_date = msg.start_date.map(|s| {
                DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(0, 0, 0)), Utc)
            });
            let end_date = msg.end_date.map(|s| {
                DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(23, 59, 59)), Utc)
            });
            client
                .get_strava_activites(start_date, end_date)
                .map_err(Into::into)
        })
        .await?
    }
}

pub struct StravaActivitiesDBRequest(pub StravaActivitiesRequest);

#[async_trait]
impl HandleRequest<StravaActivitiesDBRequest> for PgPool {
    type Result = Result<HashMap<StackString, StravaItem>, Error>;
    async fn handle(&self, msg: StravaActivitiesDBRequest) -> Self::Result {
        let start_date = msg
            .0
            .start_date
            .map(|s| DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(0, 0, 0)), Utc));
        let end_date = msg.0.end_date.map(|s| {
            DateTime::from_utc(NaiveDateTime::new(s, NaiveTime::from_hms(23, 59, 59)), Utc)
        });
        get_strava_ids(self, start_date, end_date)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaActiviesDBUpdateRequest {
    pub updates: HashMap<StackString, StravaItem>,
}

#[async_trait]
impl HandleRequest<StravaActiviesDBUpdateRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    async fn handle(&self, msg: StravaActiviesDBUpdateRequest) -> Self::Result {
        upsert_strava_id(&msg.updates, self)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaUploadRequest {
    pub filename: String,
    pub title: String,
    pub activity_type: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

#[async_trait]
impl HandleRequest<StravaUploadRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: StravaUploadRequest) -> Self::Result {
        if !Path::new(msg.filename.as_str()).exists() {
            return Ok(format!("File {} does not exist", msg.filename));
        }
        let sport = msg.activity_type.parse()?;

        let config = CONFIG.clone();

        spawn_blocking(move || {
            let client = StravaClient::from_file(config, Some(StravaAuthType::Write))?;
            client
                .upload_strava_activity(
                    &Path::new(msg.filename.as_str()),
                    &msg.title,
                    msg.description.as_ref().map_or("", String::as_str),
                    msg.is_private.unwrap_or(false),
                    sport,
                )
                .map(|id| format!("http://strava.com/activities/{}", id))
                .map_err(Into::into)
        })
        .await?
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaUpdateRequest {
    pub activity_id: String,
    pub title: String,
    pub activity_type: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

#[async_trait]
impl HandleRequest<StravaUpdateRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: StravaUpdateRequest) -> Self::Result {
        let sport = msg.activity_type.parse()?;

        let config = CONFIG.clone();

        spawn_blocking(move || {
            let client = StravaClient::from_file(config, Some(StravaAuthType::Write))?;
            client
                .update_strava_activity(
                    &msg.activity_id,
                    &msg.title,
                    msg.description.as_deref(),
                    msg.is_private,
                    sport,
                )
                .map_err(Into::into)
        })
        .await?
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
    type Result = Result<String, Error>;
    async fn handle(&self, msg: AddGarminCorrectionRequest) -> Self::Result {
        let mut corr_list = self.handle(GarminCorrRequest {}).await?;
        let filename = corr_list
            .get_filename_from_datetime(msg.start_time)
            .await?
            .ok_or_else(|| {
                format_err!(
                    "start_time {} doesn't match any existing file",
                    msg.start_time
                )
            })?;
        let unique_key = (msg.start_time, msg.lap_number);

        let mut new_corr = if let Some(corr) = corr_list.get_corr_list_map().get(&unique_key) {
            corr.clone()
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

        corr_list
            .get_corr_list_map_mut()
            .insert(unique_key, new_corr);

        corr_list.dump_corrections_to_db().await?;

        let cache_path = Path::new(CONFIG.cache_dir.as_str()).join(&format!("{}.avro", filename));
        let summary_path =
            Path::new(CONFIG.summary_cache.as_str()).join(&format!("{}.summary.avro", filename));
        remove_file(cache_path).await?;
        remove_file(summary_path).await?;

        let gcli = GarminCli::from_pool(&self)?;
        gcli.proc_everything().await?;

        Ok("".to_string())
    }
}
