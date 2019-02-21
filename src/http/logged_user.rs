use actix_web::{middleware::identity::RequestIdentity, FromRequest, HttpRequest};
use jsonwebtoken::{decode, Validation};
use std::convert::From;
use std::env;

use super::errors::ServiceError;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct LoggedUser {
    pub email: String,
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
