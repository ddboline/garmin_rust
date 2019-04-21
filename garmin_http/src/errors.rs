use failure::Fail;

use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};

#[derive(Fail, Debug)]
pub enum ServiceError {
    #[fail(display = "Internal Server Error")]
    InternalServerError,

    #[fail(display = "BadRequest: {}", _0)]
    BadRequest(String),

    #[fail(display = "Unauthorized")]
    Unauthorized,
}

// impl ResponseError trait allows to convert our errors into http responses with appropriate data
impl ResponseError for ServiceError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            ServiceError::InternalServerError => {
                HttpResponse::InternalServerError().json("Internal Server Error, Please try later")
            }
            ServiceError::BadRequest(ref message) => HttpResponse::BadRequest().json(message),
            ServiceError::Unauthorized => HttpResponse::build(StatusCode::OK)
                .content_type("text/html; charset=utf-8")
                .body(
                    include_str!("../../templates/login.html")
                        .replace("main.css", "/auth/main.css")
                        .replace("main.js", "/auth/main.js"),
                ),
        }
    }
}
