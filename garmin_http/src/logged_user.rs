pub use authorized_users::{
    get_random_key, get_secrets, token::Token, AuthorizedUser, AUTHORIZED_USERS, JWT_SECRET,
    KEY_LENGTH, SECRET_KEY, TRIGGER_DB_UPDATE,
};
use cookie::Cookie;
use log::debug;
use reqwest::{header::HeaderValue, Client};
use rweb::{filters::cookie::cookie, Filter, Rejection, Schema};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    convert::{TryFrom, TryInto},
    env::var,
    str::FromStr,
};
use uuid::Uuid;

use garmin_lib::{
    common::{garmin_config::GarminConfig, pgpool::PgPool},
    utils::garmin_util::get_authorized_users,
};

use crate::errors::ServiceError as Error;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Schema)]
pub struct LoggedUser {
    #[schema(description = "Email Address")]
    pub email: StackString,
    #[schema(description = "Session UUID")]
    pub session: Uuid,
    #[schema(description = "Secret Key")]
    pub secret_key: StackString,
}

impl LoggedUser {
    pub fn verify_session_id(&self, session_id: Uuid) -> Result<(), Error> {
        if self.session == session_id {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    pub fn filter() -> impl Filter<Extract = (Self,), Error = Rejection> + Copy {
        cookie("session-id")
            .and(cookie("jwt"))
            .and_then(|id: Uuid, user: Self| async move {
                user.verify_session_id(id)
                    .map(|_| user)
                    .map_err(rweb::reject::custom)
            })
    }

    pub async fn get_session(
        &self,
        client: &Client,
        config: &GarminConfig,
    ) -> Result<Session, anyhow::Error> {
        #[derive(Deserialize, Debug)]
        struct SessionResponse {
            history: Option<Vec<StackString>>,
        }
        let url = format!("https://{}/api/session/garmin", config.domain);
        let value = HeaderValue::from_str(&self.session.to_string())?;
        let key = HeaderValue::from_str(&self.secret_key)?;
        let session: Option<SessionResponse> = client
            .get(url)
            .header("session", value)
            .header("secret-key", key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        debug!("Got session {:?}", session);
        match session {
            Some(session) => Ok(Session {
                history: session.history.unwrap_or_else(Vec::new),
            }),
            None => Ok(Session::default()),
        }
    }

    pub async fn set_session(
        &self,
        client: &Client,
        config: &GarminConfig,
        session: &Session,
    ) -> Result<(), anyhow::Error> {
        let url = format!("https://{}/api/session/garmin", config.domain);
        let value = HeaderValue::from_str(&self.session.to_string())?;
        let key = HeaderValue::from_str(&self.secret_key)?;
        client
            .post(url)
            .header("session", value)
            .header("secret-key", key)
            .json(session)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

impl From<AuthorizedUser> for LoggedUser {
    fn from(user: AuthorizedUser) -> Self {
        Self {
            email: user.email,
            session: user.session,
            secret_key: user.secret_key,
        }
    }
}

impl TryFrom<Token> for LoggedUser {
    type Error = Error;
    fn try_from(token: Token) -> Result<Self, Self::Error> {
        let user = token.try_into()?;
        if AUTHORIZED_USERS.is_authorized(&user) {
            Ok(user.into())
        } else {
            debug!("NOT AUTHORIZED {:?}", user);
            Err(Error::Unauthorized)
        }
    }
}

impl FromStr for LoggedUser {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let token: Token = s.to_string().into();
        token.try_into()
    }
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Session {
    pub history: Vec<StackString>,
}

impl FromStr for Session {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = base64::decode(s)?;
        let history_str = String::from_utf8(data)?;
        let history = history_str.split(';').map(Into::into).collect();
        Ok(Session { history })
    }
}

impl Session {
    pub fn get_jwt_cookie(&self, domain: &str) -> Cookie<'static> {
        let history_str = self.history.join(";");
        let token = base64::encode(history_str);
        Cookie::build("session", token)
            .http_only(true)
            .path("/")
            .domain(domain.to_string())
            .finish()
    }
}

pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    debug!("{:?}", *TRIGGER_DB_UPDATE);
    let users = if TRIGGER_DB_UPDATE.check() {
        get_authorized_users(pool).await?
    } else {
        AUTHORIZED_USERS.get_users()
    };
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        AUTHORIZED_USERS.merge_users(&["user@test".into()])?;
    }
    AUTHORIZED_USERS.merge_users(&users)?;
    debug!("{:?}", *AUTHORIZED_USERS);
    Ok(())
}
