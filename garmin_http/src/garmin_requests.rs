use actix::{Handler, Message};
use chrono::{DateTime, Duration, SecondsFormat};
use failure::Error;
use std::collections::HashMap;
use std::path::Path;

use fitbit_lib::fitbit_client::FitbitClient;
use fitbit_lib::fitbit_heartrate::FitbitHeartRate;
use fitbit_lib::scale_measurement::ScaleMeasurement;

use strava_lib::strava_client::{StravaAuthType, StravaClient};

use garmin_lib::common::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::garmin_summary::get_list_of_files_from_db;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::common::strava_sync::{
    get_strava_id_maximum_begin_datetime, upsert_strava_id, StravaItem,
};

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
                    let max_datetime = DateTime::parse_from_rfc3339(&dt)?;
                    let max_datetime = max_datetime - Duration::days(14);
                    let max_datetime = max_datetime.to_rfc3339_opts(SecondsFormat::Secs, true);
                    println!("max_datetime {}", max_datetime);
                    Some(max_datetime)
                }
                None => None,
            };

            let client = StravaClient::from_file(&config, Some(StravaAuthType::Read))?;
            let activities = client.get_strava_activites(max_datetime, None)?;

            upsert_strava_id(&activities, &self)
        })
    }
}

pub struct GarminSyncRequest {}

impl Message for GarminSyncRequest {
    type Result = Result<(), Error>;
}

impl Handler<GarminSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, _: GarminSyncRequest, _: &mut Self::Context) -> Self::Result {
        let gcli = GarminCli::from_pool(&self)?;
        gcli.sync_everything()?;
        gcli.proc_everything()
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
        let client = FitbitClient::from_file(&config)?;
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
        let mut client = FitbitClient::from_file(&config)?;
        let url = client.get_fitbit_access_token(&msg.code)?;
        client.to_file()?;
        Ok(url)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateApiRequest {
    date: String,
}

impl Message for FitbitHeartrateApiRequest {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
}

impl Handler<FitbitHeartrateApiRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    fn handle(&mut self, msg: FitbitHeartrateApiRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(&config)?;
        client.get_fitbit_intraday_time_series_heartrate(&msg.date)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitHeartrateDbRequest {
    date: String,
}

impl Message for FitbitHeartrateDbRequest {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
}

impl Handler<FitbitHeartrateDbRequest> for PgPool {
    type Result = Result<Vec<FitbitHeartRate>, Error>;
    fn handle(&mut self, msg: FitbitHeartrateDbRequest, _: &mut Self::Context) -> Self::Result {
        FitbitHeartRate::read_from_db(self, &msg.date)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FitbitSyncRequest {
    date: String,
}

impl Message for FitbitSyncRequest {
    type Result = Result<(), Error>;
}

impl Handler<FitbitSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, msg: FitbitSyncRequest, _: &mut Self::Context) -> Self::Result {
        let config = GarminConfig::get_config(None)?;
        let client = FitbitClient::from_file(&config)?;
        client.import_fitbit_heartrate(&msg.date, self)?;
        Ok(())
    }
}

pub struct ScaleMeasurementRequest {}

impl Message for ScaleMeasurementRequest {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
}

impl Handler<ScaleMeasurementRequest> for PgPool {
    type Result = Result<Vec<ScaleMeasurement>, Error>;
    fn handle(&mut self, _: ScaleMeasurementRequest, _: &mut Self::Context) -> Self::Result {
        ScaleMeasurement::read_from_db(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        let client = StravaClient::from_file(&config, auth_type)?;
        client.get_authorization_url()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        let mut client = StravaClient::from_file(&config, None)?;
        client.process_callback(&msg.code, &msg.state)?;
        client.to_file()?;
        let body = r#"
            <title>Strava auth code received!</title>
            This window can be closed.
            <script language="JavaScript" type="text/javascript">window.close()</script>"#;
        Ok(body.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        let client = StravaClient::from_file(&config, Some(StravaAuthType::Read))?;
        client.get_strava_activites(msg.start_date, msg.end_date)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        let client = StravaClient::from_file(&config, Some(StravaAuthType::Write))?;
        client
            .upload_strava_activity(
                &filepath,
                &msg.title,
                msg.description.as_ref().unwrap_or(&"".to_string()),
                msg.is_private.unwrap_or(false),
                sport,
            )
            .map(|id| format!("http://strava.com/activities/{}", id))
    }
}
