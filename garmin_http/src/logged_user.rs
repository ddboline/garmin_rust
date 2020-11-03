use anyhow::Error;
use log::debug;
use stack_string::StackString;
use std::env::var;
use actix_identity::Identity;
use actix_web::{dev::Payload, FromRequest, HttpRequest};
pub use authorized_users::{
    get_random_key, get_secrets, AuthorizedUser, AUTHORIZED_USERS, JWT_SECRET, KEY_LENGTH,
    SECRET_KEY, TRIGGER_DB_UPDATE, token::Token,
};
use futures::{
    executor::block_on,
    future::{ready, Ready},
};
use serde::{Deserialize, Serialize};
use std::env;

use garmin_lib::common::pgpool::PgPool;

use crate::errors::ServiceError;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub struct LoggedUser {
    pub email: StackString,
}

impl From<AuthorizedUser> for LoggedUser {
    fn from(user: AuthorizedUser) -> Self {
        Self { email: user.email }
    }
}

impl From<LoggedUser> for AuthorizedUser {
    fn from(user: LoggedUser) -> Self {
        Self { email: user.email }
    }
}

fn _from_request(req: &HttpRequest, pl: &mut Payload) -> Result<LoggedUser, actix_web::Error> {
    if let Ok(s) = env::var("TESTENV") {
        if &s == "true" {
            return Ok(LoggedUser {
                email: "user@test".into(),
            });
        }
    }
    if let Some(identity) = block_on(Identity::from_request(req, pl))?.identity() {
        if let Some(user) = Token::decode_token(&identity.into()).ok().map(Into::into) {
            if AUTHORIZED_USERS.is_authorized(&user) {
                return Ok(user.into());
            } else {
                debug!("not authorized {:?}", user);
            }
        }
    }
    Err(ServiceError::Unauthorized.into())
}

impl FromRequest for LoggedUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, actix_web::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, pl: &mut Payload) -> Self::Future {
        ready(_from_request(req, pl))
    }
}

pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    debug!("{:?}", *TRIGGER_DB_UPDATE);
    let users = if TRIGGER_DB_UPDATE.check() {
        let query = "SELECT email FROM authorized_users";
        let results: Result<Vec<_>, Error> = pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .into_iter()
            .map(|row| {
                let email: StackString = row.try_get(0)?;
                Ok(AuthorizedUser { email })
            })
            .collect();
        results?
    } else {
        AUTHORIZED_USERS.get_users()
    };
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        let user = AuthorizedUser {
            email: "user@test".into(),
        };
        AUTHORIZED_USERS.merge_users(&[user])?;
    }
    AUTHORIZED_USERS.merge_users(&users)?;
    debug!("{:?}", *AUTHORIZED_USERS);
    Ok(())
}
