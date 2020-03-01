use actix_multipart::MultipartError;
use actix_web::{error::ResponseError, HttpResponse};
use anyhow::Error as AnyhowError;
use rust_auth_server::static_files::login_html;
use std::fmt::Debug;
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
}

// impl ResponseError trait allows to convert our errors into http responses with appropriate data
impl ResponseError for ServiceError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            Self::BadRequest(ref message) => HttpResponse::BadRequest().json(message),
            Self::Unauthorized => {
                TRIGGER_DB_UPDATE.set();
                login_html()
            }
            _ => {
                HttpResponse::InternalServerError().json("Internal Server Error, Please try later")
            }
        }
    }
}

impl From<MultipartError> for ServiceError {
    fn from(item: MultipartError) -> Self {
        Self::BlockingError(item.to_string())
    }
}
