extern crate config;
extern crate rayon;
extern crate tempdir;

use actix::{Handler, Message};
use failure::Error;

use crate::common::garmin_cli::GarminCli;
use crate::common::garmin_correction_lap::GarminCorrectionList;
use crate::common::pgpool::PgPool;
use crate::reports::garmin_report_options::GarminReportOptions;

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

fn get_list_of_files_from_db(constraints: &[String], pool: &PgPool) -> Result<Vec<String>, Error> {
    let constr = if constraints.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", constraints.join(" OR "))
    };

    let query = format!("SELECT filename FROM garmin_summary {}", constr);

    let conn = pool.get()?;

    Ok(conn
        .query(&query, &[])?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

#[derive(Debug, Default)]
pub struct GarminHtmlRequest {
    pub filter: String,
    pub history: String,
    pub options: GarminReportOptions,
    pub constraints: Vec<String>,
}

impl Message for GarminHtmlRequest {
    type Result = Result<String, Error>;
}

impl Handler<GarminHtmlRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&mut self, msg: GarminHtmlRequest, _: &mut Self::Context) -> Self::Result {
        let body = GarminCli::from_pool(&self).run_html(&msg)?;
        Ok(body)
    }
}

impl GarminHtmlRequest {
    pub fn get_list_of_files_from_db(&self, pool: &PgPool) -> Result<Vec<String>, Error> {
        get_list_of_files_from_db(&self.constraints, &pool)
    }
}

#[derive(Default)]
pub struct GarminListRequest {
    pub constraints: Vec<String>,
}

impl Into<GarminListRequest> for GarminHtmlRequest {
    fn into(self) -> GarminListRequest {
        GarminListRequest {
            constraints: self.constraints,
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
