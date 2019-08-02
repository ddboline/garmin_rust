use failure::{err_msg, Error};
use google_sheets4::{Sheet, Sheets};
use hyper::net::HttpsConnector;
use hyper::Client;
use hyper_native_tls::NativeTlsClient;
use std::fs::{create_dir_all, File};
use std::path::Path;
use std::rc::Rc;
use yup_oauth2::{
    Authenticator, ConsoleApplicationSecret, DefaultAuthenticatorDelegate, DiskTokenStorage,
    FlowType,
};

use garmin_lib::common::garmin_config::GarminConfig;

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
        let secret_file = File::open(config.google_secret_file.clone())?;
        let secret: ConsoleApplicationSecret = serde_json::from_reader(secret_file)?;
        let secret = secret
            .installed
            .ok_or_else(|| err_msg("ConsoleApplicationSecret.installed is None"))?;
        let token_file = format!("{}/{}.json", config.google_token_path, session_name);

        let parent = Path::new(&config.google_token_path);

        if !parent.exists() {
            create_dir_all(parent)?;
        }

        let auth = Authenticator::new(
            &secret,
            DefaultAuthenticatorDelegate,
            Client::with_connector(HttpsConnector::new(NativeTlsClient::new()?)),
            DiskTokenStorage::new(&token_file)?,
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
            .map_err(|e| err_msg(format!("{:#?}", e)))?;
        sheets.sheets.ok_or_else(|| err_msg("No sheets"))
    }
}

#[cfg(test)]
mod tests {
    use crate::sheets_client::SheetsClient;
    use garmin_lib::common::garmin_config::GarminConfig;

    #[test]
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
        println!("{:?}", sheets);
        if let Some(sheets) = sheets.sheets {
            let sheet = &sheets[0];

            if let Some(data) = &sheet.data {
                println!("{:?}", data);
            }
        }
        assert!(false);
    }
}
