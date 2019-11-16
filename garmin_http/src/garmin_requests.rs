use actix::{Handler, Message};
use chrono::{Duration, Local, NaiveDate, SecondsFormat};
use failure::Error;
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use fitbit_lib::fitbit_client::FitbitClient;
use fitbit_lib::fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate};
use fitbit_lib::scale_measurement::ScaleMeasurement;

use strava_lib::strava_client::{StravaAuthType, StravaClient};

use garmin_lib::common::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::garmin_file::GarminFile;
use garmin_lib::common::garmin_summary::get_list_of_files_from_db;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::common::strava_sync::{
    get_strava_id_maximum_begin_datetime, upsert_strava_id, StravaItem,
};
use garmin_lib::reports::garmin_templates::{PLOT_TEMPLATE, TIMESERIESTEMPLATE};

use super::logged_user::LoggedUser;

pub struct GarminCorrRequest {}

impl Message for GarminCorrRequest {
    type Result = Result<GarminCorrectionList, Error>;
}

impl Handler<GarminCorrRequest> for PgPool {
    type Result = Result<GarminCorrectionList, Error>;
    fn handle(&mut self, _: GarminCorrRequest, _: &mut Self::Context) -> Self::Result {
        GarminCorrectionList::from_pool(&self).read_corrections_from_db()
    }
}

pub struct GarminHtmlRequest(pub GarminRequest);

impl Message for GarminHtmlRequest {
    type Result = Result<String, Error>;
}

impl Handler<GarminHtmlRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: GarminHtmlRequest, _: &mut Self::Context) -> Self::Result {
        let body = GarminCli::from_pool(&self)?.run_html(&msg.0)?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    pub fn get_list_of_files_from_db(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
        get_list_of_files_from_db(&self.0.constraints, &pool)
    }
}

#[derive(Default)]
pub struct GarminListRequest {
    pub constraints: Vec<String>,
}

impl Into<GarminListRequest> for GarminHtmlRequest {
    fn into(self) -> GarminListRequest {
        GarminListRequest {
            constraints: self.0.constraints,
        }
    }
}

impl GarminListRequest {
    pub fn get_list_of_files_from_db(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
        get_list_of_files_from_db(&self.constraints, &pool)
    }
}

impl Message for GarminListRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<GarminListRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    fn handle(&mut self, msg: GarminListRequest, _: &mut Self::Context) -> Self::Result {
        msg.get_list_of_files_from_db(self)
    }
}

pub struct AuthorizedUserRequest {
    pub user: LoggedUser,
}

impl Message for AuthorizedUserRequest {
    type Result = Result<bool, Error>;
}

impl Handler<AuthorizedUserRequest> for PgPool {
    type Result = Result<bool, Error>;
    fn handle(&mut self, msg: AuthorizedUserRequest, _: &mut Self::Context) -> Self::Result {
        msg.user.is_authorized(self)
    }
}

#[derive(Serialize, Deserialize)]
pub struct GarminUploadRequest {
    pub filename: String,
}

impl Message for GarminUploadRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<GarminUploadRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    fn handle(&mut self, req: GarminUploadRequest, _: &mut Self::Context) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let filenames = vec![req.filename];
        gcli.process_filenames(&filenames)?;
        gcli.proc_everything()?;
        Ok(filenames)
    }
}

pub struct GarminConnectSyncRequest {}

impl Message for GarminConnectSyncRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<GarminConnectSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    fn handle(&mut self, _: GarminConnectSyncRequest, _: &mut Self::Context) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let filenames = gcli.sync_with_garmin_connect()?;
        gcli.proc_everything()?;
        Ok(filenames)
    }
}

pub struct StravaSyncRequest {}

impl Message for StravaSyncRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<StravaSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    fn handle(&mut self, _: StravaSyncRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;

        get_strava_id_maximum_begin_datetime(&self).and_then(|max_datetime| {
            let max_datetime = match max_datetime {
                Some(dt) => {
                    let max_datetime = dt - Duration::days(14);
                    let max_datetime = max_datetime.to_rfc3339_opts(SecondsFormat::Secs, true);
                    debug!("max_datetime {}", max_datetime);
                    Some(max_datetime)
                }
                None => None,
            };

            let client = StravaClient::from_file(config, Some(StravaAuthType::Read))?;
            let activities = client.get_strava_activites(max_datetime, None)?;

            upsert_strava_id(&activities, &self)
        })
    }
}

pub struct GarminSyncRequest {}

impl Message for GarminSyncRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<GarminSyncRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;
    fn handle(&mut self, _: GarminSyncRequest, _: &mut Self::Context) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        let mut output = gcli.sync_everything(false)?;
        output.extend_from_slice(&gcli.proc_everything()?);
        Ok(output)
    }
}

pub struct FitbitAuthRequest {}

impl Message for FitbitAuthRequest {
    type Result = Result<String, Error>;
}

impl Handler<FitbitAuthRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, _: FitbitAuthRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config)?;
        let url = client.get_fitbit_auth_url()?;
        Ok(url)
    }
}

#[derive(Deserialize)]
pub struct FitbitCallbackRequest {
    code: String,
}

impl Message for FitbitCallbackRequest {
    type Result = Result<String, Error>;
}

impl Handler<FitbitCallbackRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: FitbitCallbackRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let mut client = FitbitClient::from_file(config)?;
        let url = client.get_fitbit_access_token(&msg.code)?;
        client.to_file()?;
        Ok(url)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateApiRequest {
    date: NaiveDate,
}

impl Message for FitbitHeartrateApiRequest {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
}

impl Handler<FitbitHeartrateApiRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    fn handle(&mut self, msg: FitbitHeartrateApiRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config)?;
        client.get_fitbit_intraday_time_series_heartrate(msg.date)
    }
}

pub struct FitbitBodyWeightFatRequest {}

impl Message for FitbitBodyWeightFatRequest {
    type Result = Result<Vec<FitbitBodyWeightFat>, Error>;
}

impl Handler<FitbitBodyWeightFatRequest> for PgPool {
    type Result = Result<Vec<FitbitBodyWeightFat>, Error>;
    fn handle(&mut self, _: FitbitBodyWeightFatRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config)?;
        client.get_fitbit_bodyweightfat()
    }
}

pub struct FitbitBodyWeightFatUpdateRequest {}

impl Message for FitbitBodyWeightFatUpdateRequest {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
}

impl Handler<FitbitBodyWeightFatUpdateRequest> for PgPool {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
    fn handle(
        &mut self,
        _: FitbitBodyWeightFatUpdateRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        let start_date: NaiveDate = (Local::now() - Duration::days(30)).naive_local().date();
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config)?;
        let existing_map: HashMap<NaiveDate, _> = client
            .get_fitbit_bodyweightfat()?
            .into_iter()
            .map(|entry| {
                let date = entry.datetime.with_timezone(&Local).naive_local().date();
                (date, entry)
            })
            .collect();
        let new_measurements: Vec<_> =
            ScaleMeasurement::read_from_db(self, Some(start_date), None)?
                .into_iter()
                .filter(|entry| {
                    let date = entry.datetime.with_timezone(&Local).naive_local().date();
                    !existing_map.contains_key(&date)
                })
                .collect();
        client.update_fitbit_bodyweightfat(&new_measurements)?;
        Ok(new_measurements)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateDbRequest {
    date: NaiveDate,
}

impl Message for FitbitHeartrateDbRequest {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
}

impl Handler<FitbitHeartrateDbRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    fn handle(&mut self, msg: FitbitHeartrateDbRequest, _: &mut Self::Context) -> Self::Result {
        FitbitHeartRate::read_from_db(self, msg.date)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateDbUpdateRequest {
    updates: Vec<FitbitHeartRate>,
}

impl Message for FitbitHeartrateDbUpdateRequest {
    type Result = Result<(), Error>;
}

impl Handler<FitbitHeartrateDbUpdateRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(
        &mut self,
        msg: FitbitHeartrateDbUpdateRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        FitbitHeartRate::insert_slice_into_db(&msg.updates, self)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitSyncRequest {
    date: NaiveDate,
}

impl Message for FitbitSyncRequest {
    type Result = Result<(), Error>;
}

impl Handler<FitbitSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, msg: FitbitSyncRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(config)?;
        client.import_fitbit_heartrate(msg.date, self)
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

impl Message for ScaleMeasurementRequest {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
}

impl Handler<ScaleMeasurementRequest> for PgPool {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
    fn handle(&mut self, req: ScaleMeasurementRequest, _: &mut Self::Context) -> Self::Result {
        ScaleMeasurement::read_from_db(self, req.start_date, req.end_date)
    }
}

pub struct ScaleMeasurementPlotRequest(ScaleMeasurementRequest);

impl From<ScaleMeasurementRequest> for ScaleMeasurementPlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(365);
        Self(item)
    }
}

impl Message for ScaleMeasurementPlotRequest {
    type Result = Result<String, Error>;
}

impl Handler<ScaleMeasurementPlotRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, req: ScaleMeasurementPlotRequest, _: &mut Self::Context) -> Self::Result {
        let measurements = ScaleMeasurement::read_from_db(self, req.0.start_date, req.0.end_date)?;
        ScaleMeasurement::get_scale_measurement_plots(&measurements)
    }
}

pub struct FitbitHeartratePlotRequest {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

impl From<ScaleMeasurementRequest> for FitbitHeartratePlotRequest {
    fn from(item: ScaleMeasurementRequest) -> Self {
        let item = item.add_default(3);
        Self {
            start_date: item.start_date.expect("this should be impossible"),
            end_date: item.end_date.expect("this should be impossible"),
        }
    }
}

impl Message for FitbitHeartratePlotRequest {
    type Result = Result<String, Error>;
}

impl Handler<FitbitHeartratePlotRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, req: FitbitHeartratePlotRequest, _: &mut Self::Context) -> Self::Result {
        let nminutes = 5;
        let config = GarminConfig::get_config(None)?;
        let ndays = (req.end_date - req.start_date).num_days();
        let heartrate_values: Result<Vec<_>, Error> = (0..=ndays)
            .into_par_iter()
            .map(|i| {
                let mut heartrate_values = Vec::new();
                let date = req.start_date + Duration::days(i);
                let values: Vec<_> = FitbitHeartRate::read_from_db_resample(self, date, nminutes)?
                    .into_iter()
                    .map(|h| (h.datetime, h.value))
                    .collect();
                heartrate_values.extend_from_slice(&values);
                let constraint = format!("date(begin_datetime at time zone 'utc') = '{}'", date);
                for filename in get_list_of_files_from_db(&[constraint], self)? {
                    let avro_file = format!("{}/{}.avro", &config.cache_dir, filename);
                    let points: Vec<_> = GarminFile::read_avro(&avro_file)?
                        .points
                        .into_iter()
                        .filter_map(|p| p.heart_rate.map(|h| (p.time, h as i32)))
                        .collect();
                    heartrate_values.extend_from_slice(&points);
                }
                Ok(heartrate_values)
            })
            .collect();
        let heartrate_values: Vec<_> = heartrate_values?.into_iter().flatten().collect();
        let mut final_values = Vec::new();
        for (_, group) in &heartrate_values
            .into_iter()
            .group_by(|(d, _)| d.timestamp() / (nminutes as i64 * 60))
        {
            let g: Vec<_> = group.collect();
            let d = g.iter().map(|(d, _)| *d).min();
            if let Some(d) = d {
                let v = g.iter().map(|(_, v)| v).sum::<i32>() / g.len() as i32;
                let d = d.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                final_values.push((d, v));
            }
        }
        final_values.sort();
        let js_str = serde_json::to_string(&final_values).unwrap_or_else(|_| "".to_string());
        let plots = TIMESERIESTEMPLATE
            .replace("DATA", &js_str)
            .replace("EXAMPLETITLE", "Heart Rate")
            .replace("XAXIS", "Date")
            .replace("YAXIS", "Heart Rate");
        let plots = format!("<script>\n{}\n</script>", plots);
        let body = PLOT_TEMPLATE
            .replace("INSERTOTHERIMAGESHERE", &plots)
            .replace("INSERTTEXTHERE", "");
        Ok(body)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScaleMeasurementUpdateRequest {
    pub measurements: Vec<ScaleMeasurement>,
}

impl Message for ScaleMeasurementUpdateRequest {
    type Result = Result<(), Error>;
}

impl Handler<ScaleMeasurementUpdateRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(
        &mut self,
        msg: ScaleMeasurementUpdateRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        let measurement_set: HashSet<_> = ScaleMeasurement::read_from_db(self, None, None)?
            .into_iter()
            .map(|d| d.datetime)
            .collect();
        msg.measurements
            .iter()
            .map(|meas| {
                if !measurement_set.contains(&meas.datetime) {
                    meas.insert_into_db(self)?;
                }
                Ok(())
            })
            .collect()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaAuthRequest {
    pub auth_type: Option<String>,
}

impl Message for StravaAuthRequest {
    type Result = Result<String, Error>;
}

impl Handler<StravaAuthRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: StravaAuthRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let auth_type = msg.auth_type.and_then(|a| match a.as_str() {
            "read" => Some(StravaAuthType::Read),
            "write" => Some(StravaAuthType::Write),
            _ => None,
        });
        let client = StravaClient::from_file(config, auth_type)?;
        client.get_authorization_url()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaCallbackRequest {
    pub code: String,
    pub state: String,
}

impl Message for StravaCallbackRequest {
    type Result = Result<String, Error>;
}

impl Handler<StravaCallbackRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: StravaCallbackRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let mut client = StravaClient::from_file(config, None)?;
        client.process_callback(&msg.code, &msg.state)?;
        client.to_file()?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.into())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StravaActivitiesRequest {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl Message for StravaActivitiesRequest {
    type Result = Result<HashMap<String, StravaItem>, Error>;
}

impl Handler<StravaActivitiesRequest> for PgPool {
    type Result = Result<HashMap<String, StravaItem>, Error>;
    fn handle(&mut self, msg: StravaActivitiesRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::from_file(config, Some(StravaAuthType::Read))?;
        client.get_strava_activites(msg.start_date, msg.end_date)
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

impl Message for StravaUploadRequest {
    type Result = Result<String, Error>;
}

impl Handler<StravaUploadRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: StravaUploadRequest, _: &mut Self::Context) -> Self::Result {
        let filepath = Path::new(&msg.filename);
        if !filepath.exists() {
            return Ok(format!("File {} does not exist", msg.filename));
        }
        let sport = msg.activity_type.parse()?;

        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::from_file(config, Some(StravaAuthType::Write))?;
        client
            .upload_strava_activity(
                &filepath,
                &msg.title,
                msg.description.as_ref().map(|x| x.as_str()).unwrap_or(""),
                msg.is_private.unwrap_or(false),
                sport,
            )
            .map(|id| format!("http://strava.com/activities/{}", id))
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

impl Message for StravaUpdateRequest {
    type Result = Result<String, Error>;
}

impl Handler<StravaUpdateRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: StravaUpdateRequest, _: &mut Self::Context) -> Self::Result {
        let sport = msg.activity_type.parse()?;

        let config = GarminConfig::get_config(None)?;
        let client = StravaClient::from_file(config, Some(StravaAuthType::Write))?;
        client.update_strava_activity(
            &msg.activity_id,
            &msg.title,
            msg.description.as_ref().map(|x| x.as_str()),
            msg.is_private,
            sport,
        )
    }
}
