use failure::{err_msg, Error};
use maplit::hashmap;

use super::garmin_config::GarminConfig;
use super::reqwest_session::ReqwestSession;

pub struct GarminConnectClient {
    config: GarminConfig,
    session: ReqwestSession,
}

impl GarminConnectClient {
    pub fn get_session(config: GarminConfig) -> Result<Self, Error> {
        let obligatory_headers = hashmap! {
            "Referer" => "https://sync.tapiriik.com",
        };
        let garmin_signin_headers = hashmap! {
            "origin" => "https://sso.garmin.com",
        };

        let data = hashmap! {
            "username" => config.garmin_connect_email.as_str(),
            "password" => config.garmin_connect_password.as_str(),
            "_eventId" => "submit",
            "embed" => "true",
        };

        let params = hashmap! {
            "service"=> "https://connect.garmin.com/modern",
            "clientId"=> "GarminConnect",
            "gauthHost"=>"https://sso.garmin.com/sso",
            "consumeServiceTicket"=>"false",
        };

        // let session = 

        Ok(Self {
            config,
            session: ReqwestSession::new(true),
        })
    }
}
