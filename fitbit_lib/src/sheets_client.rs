use failure::{err_msg, Error};
use google_sheets4::Sheets;
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
}
