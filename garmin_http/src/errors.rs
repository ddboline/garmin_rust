use base64::DecodeError;
use handlebars::RenderError;
use log::error;
use postgres_query::Error as PqError;
use rweb::{
    http::StatusCode,
    openapi::{
        ComponentDescriptor, ComponentOrInlineSchema, Entity, Response, ResponseEntity, Responses,
    },
    reject::{InvalidHeader, MissingCookie, Reject},
    Error as WarpError, Rejection, Reply,
};
use serde::Serialize;
use std::{
    borrow::Cow,
    convert::Infallible,
    fmt::{Debug, Error as FmtError},
    string::FromUtf8Error,
};
use thiserror::Error;
use tokio::task::JoinError;

use authorized_users::errors::AuthUsersError;
use garmin_lib::errors::GarminError;

use crate::logged_user::LOGIN_HTML;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("AuthUsersError {0}")]
    AuthUsersError(#[from] AuthUsersError),
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
    #[error("FmtError {0}")]
    FmtError(#[from] FmtError),
    #[error("PqError {0}")]
    PqError(#[from] PqError),
    #[error("GarminError {0}")]
    GarminError(#[from] GarminError),
}

impl Reject for ServiceError {}

#[derive(Serialize)]
struct ErrorMessage<'a> {
    code: u16,
    message: &'a str,
}

/// # Errors
/// Never returns an error
#[allow(clippy::unused_async)]
pub async fn error_response(err: Rejection) -> Result<Box<dyn Reply>, Infallible> {
    let code: StatusCode;
    let message: &str;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "NOT FOUND";
    } else if err.find::<InvalidHeader>().is_some() {
        return Ok(Box::new(login_html()));
    } else if let Some(missing_cookie) = err.find::<MissingCookie>() {
        if missing_cookie.name() == "jwt" {
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
        message,
    });
    let reply = rweb::reply::with_status(reply, code);

    Ok(Box::new(reply))
}

fn login_html() -> impl Reply {
    rweb::reply::html(LOGIN_HTML)
}

impl Entity for ServiceError {
    fn type_name() -> Cow<'static, str> {
        rweb::http::Error::type_name()
    }
    fn describe(comp_d: &mut ComponentDescriptor) -> ComponentOrInlineSchema {
        rweb::http::Error::describe(comp_d)
    }
}

impl ResponseEntity for ServiceError {
    fn describe_responses(_: &mut ComponentDescriptor) -> Responses {
        let mut map = Responses::new();

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
    use rweb::Reply;

    use garmin_lib::errors::GarminError as Error;

    use crate::errors::{error_response, ServiceError};

    #[tokio::test]
    async fn test_service_error() -> Result<(), Error> {
        let err = ServiceError::BadRequest("TEST ERROR".into()).into();
        let resp = error_response(err).await.unwrap().into_response();
        assert_eq!(resp.status().as_u16(), 400);

        let err = ServiceError::InternalServerError.into();
        let resp = error_response(err).await.unwrap().into_response();
        assert_eq!(resp.status().as_u16(), 500);
        Ok(())
    }
}
