use anyhow::Error as AnyhowError;
use base64::DecodeError;
use handlebars::RenderError;
use http::StatusCode;
use indexmap::IndexMap;
use log::error;
use rweb::{
    openapi::{Entity, Response, ResponseEntity, Responses, Schema},
    reject::{InvalidHeader, MissingCookie, Reject},
    Error as WarpError, Rejection, Reply,
};
use serde::Serialize;
use std::{borrow::Cow, convert::Infallible, fmt::Debug, string::FromUtf8Error};
use thiserror::Error;
use tokio::task::JoinError;

use crate::logged_user::TRIGGER_DB_UPDATE;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Anyhow error {0}")]
    AnyhowError(#[from] AnyhowError),
    #[error("io Error {0}")]
    IoError(#[from] std::io::Error),
    #[error("blocking error {0}")]
    BlockingError(String),
    #[error("tokio join error {0}")]
    JoinError(#[from] JoinError),
    #[error("handlebars RenderError {0}")]
    RenderError(#[from] RenderError),
    #[error("Base64DecodeError {0}")]
    DecodeError(#[from] DecodeError),
    #[error("FromUtf8Error {0}")]
    FromUtf8Error(#[from] FromUtf8Error),
    #[error("WarpError {0}")]
    WarpError(#[from] WarpError),
}

impl Reject for ServiceError {}

#[derive(Serialize)]
struct ErrorMessage {
    code: u16,
    message: String,
}

pub async fn error_response(err: Rejection) -> Result<Box<dyn Reply>, Infallible> {
    let code: StatusCode;
    let message: &str;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "NOT FOUND";
    } else if err.find::<InvalidHeader>().is_some() {
        TRIGGER_DB_UPDATE.set();
        return Ok(Box::new(login_html()));
    } else if let Some(missing_cookie) = err.find::<MissingCookie>() {
        if missing_cookie.name() == "jwt" {
            TRIGGER_DB_UPDATE.set();
            return Ok(Box::new(login_html()));
        }
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error";
    } else if let Some(service_err) = err.find::<ServiceError>() {
        match service_err {
            ServiceError::BadRequest(msg) => {
                code = StatusCode::BAD_REQUEST;
                message = msg.as_str();
            }
            ServiceError::Unauthorized => {
                TRIGGER_DB_UPDATE.set();
                return Ok(Box::new(login_html()));
            }
            _ => {
                error!("Other error: {:?}", service_err);
                code = StatusCode::INTERNAL_SERVER_ERROR;
                message = "Internal Server Error, Please try again later";
            }
        }
    } else if err.find::<rweb::reject::MethodNotAllowed>().is_some() {
        code = StatusCode::METHOD_NOT_ALLOWED;
        message = "METHOD NOT ALLOWED";
    } else {
        error!("Unknown error: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error, Please try again later";
    };

    let reply = rweb::reply::json(&ErrorMessage {
        code: code.as_u16(),
        message: message.to_string(),
    });
    let reply = rweb::reply::with_status(reply, code);

    Ok(Box::new(reply))
}

fn login_html() -> impl Reply {
    rweb::reply::html(
        "
            <script>
                !function() {
                    let final_url = location.href;
                    location.replace('/auth/login.html?final_url=' + final_url);
                }()
            </script>
        ",
    )
}

impl Entity for ServiceError {
    fn describe() -> Schema {
        rweb::http::Error::describe()
    }
}

impl ResponseEntity for ServiceError {
    fn describe_responses() -> Responses {
        let mut map = IndexMap::new();

        let error_responses = [
            (StatusCode::NOT_FOUND, "Not Found"),
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"),
            (StatusCode::BAD_REQUEST, "Bad Request"),
            (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed"),
        ];

        for (code, msg) in &error_responses {
            map.insert(
                Cow::Owned(code.as_str().into()),
                Response {
                    description: Cow::Borrowed(*msg),
                    ..Response::default()
                },
            );
        }

        map
    }
}

#[cfg(test)]
mod test {
    use anyhow::Error;
    use rweb::Reply;

    use crate::errors::{error_response, ServiceError};

    #[tokio::test]
    async fn test_service_error() -> Result<(), Error> {
        let err = ServiceError::BadRequest("TEST ERROR".into()).into();
        let resp = error_response(err).await?.into_response();
        assert_eq!(resp.status().as_u16(), 400);

        let err = ServiceError::InternalServerError.into();
        let resp = error_response(err).await?.into_response();
        assert_eq!(resp.status().as_u16(), 500);
        Ok(())
    }
}
