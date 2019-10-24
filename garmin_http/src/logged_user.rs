use actix_identity::Identity;
use actix_web::{dev::Payload, FromRequest, HttpRequest};
use chrono::{DateTime, Utc};
use failure::{err_msg, Error};
use jsonwebtoken::{decode, Validation};
use lazy_static::lazy_static;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::convert::From;
use std::env;

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::row_index_trait::RowIndexTrait;

use super::errors::ServiceError;

lazy_static! {
    pub static ref AUTHORIZED_USERS: AuthorizedUsers = AuthorizedUsers::new();
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    // issuer
    iss: String,
    // subject
    sub: String,
    //issued at
    iat: i64,
    // expiry
    exp: i64,
    // user email
    email: String,
}

impl From<Claims> for LoggedUser {
    fn from(claims: Claims) -> Self {
        LoggedUser {
            email: claims.email,
        }
    }
}

fn get_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| "my secret".into())
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub struct LoggedUser {
    pub email: String,
}

impl LoggedUser {
    pub fn is_authorized(&self, pool: &PgPool) -> Result<bool, Error> {
        let query = "SELECT count(*) FROM authorized_users WHERE email = $1";
        pool.get()?
            .query(query, &[&self.email])?
            .iter()
            .nth(0)
            .map(|row| {
                let count: i64 = row.get_idx(0)?;
                Ok(count > 0)
            })
            .ok_or_else(|| err_msg("User not found"))
            .and_then(|x| x)
    }
}

impl FromRequest for LoggedUser {
    type Error = actix_web::Error;
    type Future = Result<LoggedUser, actix_web::Error>;
    type Config = ();

    fn from_request(req: &HttpRequest, pl: &mut Payload) -> Self::Future {
        if let Some(identity) = Identity::from_request(req, pl)?.identity() {
            let user: LoggedUser = decode_token(&identity)?;
            if AUTHORIZED_USERS.is_authorized(&user) {
                return Ok(user);
            }
        }
        Err(ServiceError::Unauthorized.into())
    }
}

pub fn decode_token(token: &str) -> Result<LoggedUser, ServiceError> {
    decode::<Claims>(token, get_secret().as_ref(), &Validation::default())
        .map(|data| Ok(data.claims.into()))
        .map_err(|_err| ServiceError::Unauthorized)?
}

#[derive(Clone, Debug, Copy)]
enum AuthStatus {
    Authorized(DateTime<Utc>),
    NotAuthorized,
}

#[derive(Debug, Default)]
pub struct AuthorizedUsers(RwLock<HashMap<LoggedUser, AuthStatus>>);

impl AuthorizedUsers {
    pub fn new() -> AuthorizedUsers {
        AuthorizedUsers(RwLock::new(HashMap::new()))
    }

    pub fn fill_from_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = "SELECT email FROM authorized_users";
        let results: Result<HashSet<_>, Error> = pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let email: String = row.get_idx(0)?;
                Ok(LoggedUser { email })
            })
            .collect();
        let users = results?;
        let cached_users = self.list_of_users();

        for user in &users {
            self.store_auth(user, true)?;
        }

        for user in &cached_users {
            if !users.contains(user) {
                self.store_auth(user, false)?;
            }
        }

        Ok(())
    }

    pub fn list_of_users(&self) -> HashSet<LoggedUser> {
        self.0.read().keys().cloned().collect()
    }

    pub fn is_authorized(&self, user: &LoggedUser) -> bool {
        if let Ok(s) = env::var("TESTENV") {
            if &s == "true" {
                return true;
            }
        }
        if let Some(AuthStatus::Authorized(last_time)) = self.0.read().get(user) {
            let current_time = Utc::now();
            if (current_time - *last_time).num_minutes() < 15 {
                return true;
            }
        }
        false
    }

    pub fn cache_authorization(&self, user: &LoggedUser, pool: &PgPool) -> Result<(), Error> {
        if self.is_authorized(user) {
            Ok(())
        } else {
            user.is_authorized(pool)
                .and_then(|s| self.store_auth(user, s))
        }
    }

    pub fn store_auth(&self, user: &LoggedUser, is_auth: bool) -> Result<(), Error> {
        let current_time = Utc::now();
        let status = if is_auth {
            AuthStatus::Authorized(current_time)
        } else {
            AuthStatus::NotAuthorized
        };
        self.0.write().insert(user.clone(), status);
        Ok(())
    }
}
