use actix::{Handler, Message};
use failure::Error;

use garmin_lib::common::garmin_cli::{GarminCli, GarminRequest};
use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
use garmin_lib::common::garmin_summary::get_list_of_files_from_db;
use garmin_lib::common::pgpool::PgPool;

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

pub struct GarminSyncRequest {}

impl Message for GarminSyncRequest {
    type Result = Result<(), Error>;
}

impl Handler<GarminSyncRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, _: GarminSyncRequest, _: &mut Self::Context) -> Self::Result {
        GarminCli::from_pool(&self)?.sync_everything()
    }
}
