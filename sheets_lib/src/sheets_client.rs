use anyhow::{format_err, Error};
use chrono::{offset::TimeZone, DateTime, Utc};
use futures::future::try_join_all;
use google_sheets4::{RowData, Sheet, Sheets};
use hyper::{net::HttpsConnector, Client};
use hyper_native_tls::NativeTlsClient;
use log::debug;
use std::{
    collections::HashMap,
    fs::{create_dir_all, File},
    io::{stdout, Write},
    rc::Rc,
    sync::Arc,
};
use yup_oauth2::{
    Authenticator, ConsoleApplicationSecret, DefaultAuthenticatorDelegate, DiskTokenStorage,
    FlowType,
};

use fitbit_lib::scale_measurement::ScaleMeasurement;
use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

type GAuthenticator = Authenticator<DefaultAuthenticatorDelegate, DiskTokenStorage, Client>;
type GSheets = Sheets<Client, GAuthenticator>;

#[derive(Clone)]
pub struct SheetsClient {
    pub gsheets: Rc<GSheets>,
}

impl SheetsClient {
    pub fn new(config: &GarminConfig, session_name: &str) -> Self {
        Self {
            gsheets: Rc::new(Self::create_drive(&config, session_name).unwrap()),
        }
    }

    fn create_drive_auth(
        config: &GarminConfig,
        session_name: &str,
    ) -> Result<GAuthenticator, Error> {
        let secret_file = File::open(&config.google_secret_file)?;
        let secret: ConsoleApplicationSecret = serde_json::from_reader(secret_file)?;
        let secret = secret
            .installed
            .ok_or_else(|| format_err!("ConsoleApplicationSecret.installed is None"))?;
        let token_file = config
            .google_token_path
            .join(format!("{}.json", session_name));

        let parent = &config.google_token_path;

        if !parent.exists() {
            create_dir_all(parent)?;
        }

        let auth = Authenticator::new(
            &secret,
            DefaultAuthenticatorDelegate,
            Client::with_connector(HttpsConnector::new(NativeTlsClient::new()?)),
            DiskTokenStorage::new(&token_file.to_string_lossy().to_string())?,
            // Some(FlowType::InstalledInteractive),
            Some(FlowType::InstalledRedirect(8081)),
        );

        Ok(auth)
    }

    /// Creates a drive hub.
    fn create_drive(config: &GarminConfig, session_name: &str) -> Result<GSheets, Error> {
        let auth = Self::create_drive_auth(config, session_name)?;
        Ok(Sheets::new(
            Client::with_connector(HttpsConnector::new(NativeTlsClient::new()?)),
            auth,
        ))
    }

    pub fn get_sheets(&self, sheet_id: &str) -> Result<Vec<Sheet>, Error> {
        let (_, sheets) = self
            .gsheets
            .spreadsheets()
            .get(sheet_id)
            .include_grid_data(true)
            .doit()
            .map_err(|e| format_err!("{:#?}", e))?;
        sheets.sheets.ok_or_else(|| format_err!("No sheets"))
    }
}

pub async fn run_sync_sheets(config: &GarminConfig, pool: &PgPool) -> Result<(), Error> {
    let current_measurements: HashMap<_, _> = ScaleMeasurement::read_from_db(pool, None, None)
        .await?
        .into_iter()
        .map(|meas| (meas.datetime, meas))
        .collect();
    let current_measurements = Arc::new(current_measurements);

    let c = SheetsClient::new(config, "ddboline@gmail.com");
    let sheets = c.get_sheets("1MG8so2pFKoOIpt0Vo9pUAtoNk-Y1SnHq9DiEFi-m5Uw")?;
    let sheet = &sheets[0];
    let data = sheet.data.as_ref().ok_or_else(|| format_err!("No data"))?;
    let row_data = &data[0]
        .row_data
        .as_ref()
        .ok_or_else(|| format_err!("No row_data"))?;
    let measurements: Vec<ScaleMeasurement> = row_data[1..]
        .iter()
        .filter_map(|row| measurement_from_row_data(row).ok())
        .collect();
    writeln!(
        stdout(),
        "{} {} {}",
        data.len(),
        row_data.len(),
        measurements.len()
    )?;
    let futures = measurements.into_iter().map(|meas| {
        let current_measurements = current_measurements.clone();
        async move {
            if current_measurements.contains_key(&meas.datetime) {
                writeln!(stdout(), "exists {:?}", meas)?;
            } else {
                writeln!(stdout(), "insert {:?}", meas)?;
                meas.insert_into_db(pool).await?;
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;
    Ok(())
}

fn measurement_from_row_data(row_data: &RowData) -> Result<ScaleMeasurement, Error> {
    let values = row_data
        .values
        .as_ref()
        .ok_or_else(|| format_err!("No values"))?;
    let values: Vec<_> = values
        .iter()
        .filter_map(|x| x.formatted_value.as_deref())
        .collect();
    if values.len() > 5 {
        let datetime = Utc
            .datetime_from_str(&values[0], "%_m/%e/%Y %k:%M:%S")
            .or_else(|_| DateTime::parse_from_rfc3339(&values[0]).map(|d| d.with_timezone(&Utc)))
            .or_else(|_| {
                DateTime::parse_from_rfc3339(&values[0].replace(" ", "T"))
                    .map(|d| d.with_timezone(&Utc))
            })
            .or_else(|e| {
                debug!("{} {}", values[0], e);
                Err(e)
            })?;
        let mass: f64 = values[1].parse()?;
        let fat_pct: f64 = values[2].parse()?;
        let water_pct: f64 = values[3].parse()?;
        let muscle_pct: f64 = values[4].parse()?;
        let bone_pct: f64 = values[5].parse()?;
        Ok(ScaleMeasurement {
            datetime,
            mass,
            fat_pct,
            water_pct,
            muscle_pct,
            bone_pct,
        })
    } else {
        Err(format_err!("Too few entries"))
    }
}

#[cfg(test)]
mod tests {
    use crate::sheets_client::SheetsClient;
    use garmin_lib::common::garmin_config::GarminConfig;

    #[test]
    #[ignore]
    fn test_sheets_client() {
        let config = GarminConfig::get_config(None).unwrap();
        let c = SheetsClient::new(&config, "ddboline@gmail.com");
        let (_, sheets) = c
            .gsheets
            .spreadsheets()
            .get("1MG8so2pFKoOIpt0Vo9pUAtoNk-Y1SnHq9DiEFi-m5Uw")
            .include_grid_data(true)
            .doit()
            .unwrap();
        assert!(sheets.sheets.is_some());
    }
}
