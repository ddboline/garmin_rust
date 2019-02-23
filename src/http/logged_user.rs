use actix_web::{middleware::identity::RequestIdentity, FromRequest, HttpRequest};
use failure::{err_msg, Error};
use jsonwebtoken::{decode, Validation};
use std::collections::HashSet;
use std::convert::From;
use std::env;
use std::sync::{Arc, RwLock};

use super::errors::ServiceError;
use crate::common::pgpool::PgPool;

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
                let count: i64 = row.get(0);
                count > 0
            })
            .ok_or_else(|| err_msg("User not found"))
    }
}

impl<S> FromRequest<S> for LoggedUser {
    type Config = ();
    type Result = Result<LoggedUser, ServiceError>;
    fn from_request(req: &HttpRequest<S>, _: &Self::Config) -> Self::Result {
        if let Some(identity) = req.identity() {
            let user: LoggedUser = decode_token(&identity)?;
            return Ok(user);
        }
        Err(ServiceError::Unauthorized)
    }
}

pub fn decode_token(token: &str) -> Result<LoggedUser, ServiceError> {
    decode::<Claims>(token, get_secret().as_ref(), &Validation::default())
        .map(|data| Ok(data.claims.into()))
        .map_err(|_err| ServiceError::Unauthorized)?
}

#[derive(Clone, Debug, Default)]
pub struct AuthorizedUsers(Arc<RwLock<HashSet<LoggedUser>>>);

impl AuthorizedUsers {
    pub fn new() -> AuthorizedUsers {
        AuthorizedUsers(Arc::new(RwLock::new(HashSet::new())))
    }

    pub fn is_authorized(&self, user: &LoggedUser) -> bool {
        if let Ok(user_list) = self.0.read() {
            user_list.contains(user)
        } else {
            false
        }
    }

    pub fn store_auth(&self, user: LoggedUser) -> Result<(), Error> {
        if let Ok(mut user_list) = self.0.write() {
            user_list.insert(user);
            return Ok(());
        }
        Err(err_msg("Failed to store credentials"))
    }
}
