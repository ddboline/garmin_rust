pub use authorized_users::{
    get_random_key, get_secrets, token::Token, AuthorizedUser, AUTHORIZED_USERS, JWT_SECRET,
    KEY_LENGTH, SECRET_KEY, TRIGGER_DB_UPDATE,
};
use cookie::Cookie;
use log::debug;
use maplit::hashset;
use reqwest::Client;
use rweb::{filters::cookie::cookie, Filter, Rejection, Schema};
use rweb_helper::UuidWrapper;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    convert::{TryFrom, TryInto},
    env::var,
    str::FromStr,
};
use url::Url;
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
    pub session: UuidWrapper,
    #[schema(description = "Secret Key")]
    pub secret_key: StackString,
}

impl LoggedUser {
    /// # Errors
    /// Returns error if `session_id` does not match `self.session`
    pub fn verify_session_id(&self, session_id: Uuid) -> Result<(), Error> {
        if self.session == session_id {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    #[must_use]
    pub fn filter() -> impl Filter<Extract = (Self,), Error = Rejection> + Copy {
        cookie("session-id")
            .and(cookie("jwt"))
            .and_then(|id: Uuid, user: Self| async move {
                user.verify_session_id(id)
                    .map(|_| user)
                    .map_err(rweb::reject::custom)
            })
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn get_session(
        &self,
        client: &Client,
        config: &GarminConfig,
    ) -> Result<Session, anyhow::Error> {
        #[derive(Deserialize, Debug)]
        struct SessionResponse {
            history: Option<Vec<StackString>>,
        }

        let base_url: Url = format_sstr!("https://{}", config.domain).parse()?;
        let session: Option<SessionResponse> = AuthorizedUser::get_session_data(
            &base_url,
            self.session.into(),
            &self.secret_key,
            client,
            "garmin",
        )
        .await?;

        debug!("Got session {:?}", session);
        match session {
            Some(session) => Ok(Session {
                history: session.history.unwrap_or_default(),
            }),
            None => Ok(Session::default()),
        }
    }

    /// # Errors
    /// Returns error if api call fails
    pub async fn set_session(
        &self,
        client: &Client,
        config: &GarminConfig,
        session: &Session,
    ) -> Result<(), anyhow::Error> {
        let base_url: Url = format_sstr!("https://{}", config.domain).parse()?;
        AuthorizedUser::set_session_data(
            &base_url,
            self.session.into(),
            &self.secret_key,
            client,
            "garmin",
            session,
        )
        .await?;
        Ok(())
    }
}

impl From<AuthorizedUser> for LoggedUser {
    fn from(user: AuthorizedUser) -> Self {
        Self {
            email: user.email,
            session: user.session.into(),
            secret_key: user.secret_key,
        }
    }
}

impl TryFrom<Token> for LoggedUser {
    type Error = Error;
    fn try_from(token: Token) -> Result<Self, Self::Error> {
        if let Ok(user) = token.try_into() {
            if AUTHORIZED_USERS.is_authorized(&user) {
                return Ok(user.into());
            }
            debug!("NOT AUTHORIZED {:?}", user);
        }
        Err(Error::Unauthorized)
    }
}

impl FromStr for LoggedUser {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = StackString::new();
        buf.push_str(s);
        let token: Token = buf.into();
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
    #[must_use]
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

/// # Errors
/// Returns error if api call fails
pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    debug!("{:?}", *TRIGGER_DB_UPDATE);
    let users = if TRIGGER_DB_UPDATE.check() {
        get_authorized_users(pool).await?
    } else {
        AUTHORIZED_USERS.get_users()
    };
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        AUTHORIZED_USERS.update_users(hashset! {"user@test".into()});
    }
    AUTHORIZED_USERS.update_users(users);
    debug!("{:?}", *AUTHORIZED_USERS);
    Ok(())
}
