use axum::{
    extract::{multipart::MultipartError, Json},
    http::{
        header::{InvalidHeaderName, InvalidHeaderValue, ToStrError, CONTENT_TYPE},
        StatusCode,
    },
    response::{IntoResponse, Response},
};
use base64::DecodeError;
use handlebars::RenderError;
use log::error;
use postgres_query::Error as PqError;
use reqwest::Error as ReqwestError;
use serde::Serialize;
use stack_string::{format_sstr, StackString};
use std::{
    fmt::{Debug, Error as FmtError},
    net::AddrParseError,
    string::FromUtf8Error,
};
use thiserror::Error;
use tokio::task::JoinError;
use utoipa::{
    openapi::{
        content::ContentBuilder,
        response::{ResponseBuilder, ResponsesBuilder},
    },
    IntoResponses, PartialSchema, ToSchema,
};
use uuid::Error as ParseError;

use authorized_users::errors::AuthUsersError;
use garmin_lib::errors::GarminError;

use crate::logged_user::LOGIN_HTML;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("ReqwestError {0}")]
    ReqwestError(#[from] ReqwestError),
    #[error("AddrParseError {0}")]
    AddrParseError(#[from] AddrParseError),
    #[error("MultipartError {0}")]
    MultipartError(Box<MultipartError>),
    #[error("ToStrError {0}")]
    ToStrError(#[from] ToStrError),
    #[error("InvalidHeaderValue {0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),
    #[error("InvalidHeaderName {0}")]
    InvalidHeaderName(#[from] InvalidHeaderName),
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(StackString),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("AuthUsersError {0}")]
    AuthUsersError(#[from] AuthUsersError),
    #[error("io Error {0}")]
    IoError(#[from] std::io::Error),
    #[error("blocking error {0}")]
    BlockingError(StackString),
    #[error("tokio join error {0}")]
    JoinError(#[from] JoinError),
    #[error("handlebars RenderError {0}")]
    RenderError(Box<RenderError>),
    #[error("Base64DecodeError {0}")]
    DecodeError(#[from] DecodeError),
    #[error("FromUtf8Error {0}")]
    FromUtf8Error(Box<FromUtf8Error>),
    #[error("FmtError {0}")]
    FmtError(#[from] FmtError),
    #[error("PqError {0}")]
    PqError(Box<PqError>),
    #[error("GarminError {0}")]
    GarminError(Box<GarminError>),
}

// we can return early in our handlers if UUID provided by the user is not valid
// and provide a custom message
impl From<ParseError> for ServiceError {
    fn from(e: ParseError) -> Self {
        error!("Invalid UUID {e:?}");
        Self::BadRequest("Parse Error".into())
    }
}

impl From<RenderError> for ServiceError {
    fn from(value: RenderError) -> Self {
        Self::RenderError(value.into())
    }
}

impl From<PqError> for ServiceError {
    fn from(value: PqError) -> Self {
        Self::PqError(value.into())
    }
}

impl From<MultipartError> for ServiceError {
    fn from(value: MultipartError) -> Self {
        Self::MultipartError(value.into())
    }
}

impl From<FromUtf8Error> for ServiceError {
    fn from(value: FromUtf8Error) -> Self {
        Self::FromUtf8Error(value.into())
    }
}

impl From<GarminError> for ServiceError {
    fn from(value: GarminError) -> Self {
        Self::GarminError(value.into())
    }
}

#[derive(Serialize, ToSchema)]
struct ErrorMessage {
    #[schema(inline)]
    message: StackString,
}

impl IntoResponse for ErrorMessage {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => (
                StatusCode::OK,
                [(CONTENT_TYPE, mime::TEXT_HTML.essence_str())],
                LOGIN_HTML,
            )
                .into_response(),
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                [(CONTENT_TYPE, mime::APPLICATION_JSON.essence_str())],
                ErrorMessage { message },
            )
                .into_response(),
            e => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CONTENT_TYPE, mime::APPLICATION_JSON.essence_str())],
                ErrorMessage {
                    message: format_sstr!("Internal Server Error: {e}"),
                },
            )
                .into_response(),
        }
    }
}

impl IntoResponses for ServiceError {
    fn responses() -> std::collections::BTreeMap<
        String,
        utoipa::openapi::RefOr<utoipa::openapi::response::Response>,
    > {
        let error_message_content = ContentBuilder::new()
            .schema(Some(ErrorMessage::schema()))
            .build();
        ResponsesBuilder::new()
            .response(
                StatusCode::UNAUTHORIZED.as_str(),
                ResponseBuilder::new()
                    .description("Not Authorized")
                    .content(
                        mime::TEXT_HTML.essence_str(),
                        ContentBuilder::new().schema(Some(String::schema())).build(),
                    ),
            )
            .response(
                StatusCode::BAD_REQUEST.as_str(),
                ResponseBuilder::new().description("Bad Request").content(
                    mime::APPLICATION_JSON.essence_str(),
                    error_message_content.clone(),
                ),
            )
            .response(
                StatusCode::INTERNAL_SERVER_ERROR.as_str(),
                ResponseBuilder::new()
                    .description("Internal Server Error")
                    .content(
                        mime::APPLICATION_JSON.essence_str(),
                        error_message_content.clone(),
                    ),
            )
            .build()
            .into()
    }
}

#[cfg(test)]
mod test {
    use axum::http::header::InvalidHeaderName;
    use handlebars::RenderError;
    use postgres_query::Error as PqError;
    use reqwest::Error as ReqwestError;
    use serde_json::Error as SerdeJsonError;
    use serde_yaml_ng::Error as SerdeYamlError;
    use stack_string::StackString;
    use std::{fmt::Error as FmtError, net::AddrParseError};
    use time_tz::system::Error as TzError;
    use tokio::task::JoinError;

    use axum::{
        extract::multipart::MultipartError,
        http::header::{InvalidHeaderValue, ToStrError},
    };
    use base64::DecodeError;
    use std::string::FromUtf8Error;

    use authorized_users::errors::AuthUsersError;
    use garmin_lib::errors::GarminError;

    use crate::errors::ServiceError as Error;

    #[test]
    fn test_error_size() {
        println!("JoinError {}", std::mem::size_of::<JoinError>());
        println!("BadRequest: {}", std::mem::size_of::<StackString>());
        println!("io Error {}", std::mem::size_of::<std::io::Error>());
        println!("tokio join error {}", std::mem::size_of::<JoinError>());
        println!("TzError {}", std::mem::size_of::<TzError>());
        println!("PqError {}", std::mem::size_of::<PqError>());
        println!("FmtError {}", std::mem::size_of::<FmtError>());
        println!("io Error {}", std::mem::size_of::<std::io::Error>());
        println!(
            "InvalidHeaderName {}",
            std::mem::size_of::<InvalidHeaderName>()
        );
        println!("AuthUsersError {}", std::mem::size_of::<AuthUsersError>());
        println!("AddrParseError {}", std::mem::size_of::<AddrParseError>());
        println!("SerdeYamlError {}", std::mem::size_of::<SerdeYamlError>());
        println!("SerdeJsonError {}", std::mem::size_of::<SerdeJsonError>());
        println!(
            "Handlebars RenderError {}",
            std::mem::size_of::<RenderError>()
        );
        println!("PqError {}", std::mem::size_of::<PqError>());
        println!("FmtError {}", std::mem::size_of::<FmtError>());

        println!("AddrParseError {}", std::mem::size_of::<AddrParseError>());
        println!("MultipartError {}", std::mem::size_of::<MultipartError>());
        println!("ToStrError {}", std::mem::size_of::<ToStrError>());
        println!(
            "InvalidHeaderValue {}",
            std::mem::size_of::<InvalidHeaderValue>()
        );
        println!(
            "InvalidHeaderName {}",
            std::mem::size_of::<InvalidHeaderName>()
        );
        println!("AuthUsersError {}", std::mem::size_of::<AuthUsersError>());
        println!("Base64DecodeError {}", std::mem::size_of::<DecodeError>());
        println!("FromUtf8Error {}", std::mem::size_of::<FromUtf8Error>());
        println!("GarminError {}", std::mem::size_of::<GarminError>());
        println!("ReqwestError {}", std::mem::size_of::<ReqwestError>());

        assert_eq!(std::mem::size_of::<Error>(), 32);
    }
}
