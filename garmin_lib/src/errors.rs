use apache_avro::Error as ApacheAvroError;
use aws_sdk_s3::operation::{
    get_object::GetObjectError, list_objects::ListObjectsError, put_object::PutObjectError,
};
use aws_smithy_runtime_api::client::result::SdkError;
use aws_smithy_types::byte_stream::error::Error as AwsByteStreamError;
use base64::DecodeError;
use deadpool_postgres::{BuildError as DeadpoolBuildError, ConfigError as DeadpoolConfigError};
use envy::Error as EnvyError;
use fitparser::Error as FitParserError;
use glob::{GlobError, PatternError};
use http::header::InvalidHeaderValue;
use json::Error as JsonError;
use log::error;
use notify::Error as NotifyError;
use polars::error::PolarsError;
use postgres_query::{extract::Error as PqExtractError, Error as PqError};
use rand::distr::uniform::Error as RandUniformError;
use refinery::Error as RefineryError;
use reqwest::Error as ReqwestError;
use reqwest_oauth1::Error as ReqwestOauth1Error;
use roxmltree::Error as RoXmlTreeError;
use serde_json::Error as SerdeJsonError;
use serde_yml::Error as YamlError;
use stack_string::StackString;
use std::{
    fmt::{Debug, Error as FmtError},
    net::AddrParseError,
    num::{ParseFloatError, ParseIntError, TryFromIntError},
    str::Utf8Error,
    string::FromUtf8Error,
    time::SystemTimeError,
};
use stdout_channel::StdoutChannelError;
use subprocess::PopenError;
use telegram_bot::Error as TelegramBotError;
use thiserror::Error;
use time::error::{
    ComponentRange as TimeComponentRange, Format as TimeFormatError, Parse as TimeParseError,
};
use tokio::task::JoinError;
use tokio_postgres::error::Error as TokioPostgresError;
use url::ParseError as UrlParseError;
use uuid::Error as UuidError;
use zip::result::ZipError;

type DeadPoolError = deadpool::managed::PoolError<TokioPostgresError>;
type AwsListObjectError = SdkError<ListObjectsError, aws_smithy_runtime_api::http::Response>;
type AwsGetObjectError = SdkError<GetObjectError, aws_smithy_runtime_api::http::Response>;
type AwsPutObjectError = SdkError<PutObjectError, aws_smithy_runtime_api::http::Response>;

#[derive(Error, Debug)]
pub enum GarminError {
    #[error("AddrParseError {0}")]
    AddrParseError(#[from] AddrParseError),
    #[error("YamlError {0}")]
    YamlError(#[from] YamlError),
    #[error("NotifyError {0}")]
    NotifyError(#[from] NotifyError),
    #[error("RefineryError {0}")]
    RefineryError(#[from] RefineryError),
    #[error("StdoutChannelError {0}")]
    StdoutChannelError(#[from] StdoutChannelError),
    #[error("ReqwestOauth1Error {0}")]
    ReqwestOauth1Error(#[from] ReqwestOauth1Error),
    #[error("UuidError {0}")]
    UuidError(#[from] UuidError),
    #[error("GlobError {0}")]
    GlobError(#[from] GlobError),
    #[error("PatternError {0}")]
    PatternError(#[from] PatternError),
    #[error("PolarsError {0}")]
    PolarsError(#[from] PolarsError),
    #[error("TimeComponentRange {0}")]
    TimeComponentRange(#[from] TimeComponentRange),
    #[error("TelegramBotError {0}")]
    TelegramBotError(#[from] TelegramBotError),
    #[error("TimeFormatError {0}")]
    TimeFormatError(#[from] TimeFormatError),
    #[error("InvalidHeaderValue {0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),
    #[error("ReqwestError {0}")]
    ReqwestError(#[from] ReqwestError),
    #[error("RoXmlTreeError {0}")]
    RoXmlTreeError(#[from] RoXmlTreeError),
    #[error("FitParserError {0}")]
    FitParserError(#[from] FitParserError),
    #[error("SystemTimeError {0}")]
    SystemTimeError(#[from] SystemTimeError),
    #[error("AwsByteStreamError {0}")]
    AwsByteStreamError(#[from] AwsByteStreamError),
    #[error("AwsGetObjectError {0}")]
    AwsGetObjectError(#[from] AwsGetObjectError),
    #[error("AwsListObjectError {0}")]
    AwsListObjectError(#[from] AwsListObjectError),
    #[error("AwsPutObjectError {0}")]
    AwsPutObjectError(#[from] AwsPutObjectError),
    #[error("ApacheAvroError {0}")]
    ApacheAvroError(#[from] ApacheAvroError),
    #[error("JsonError {0}")]
    JsonError(#[from] JsonError),
    #[error("SerdeJsonError {0}")]
    SerdeJsonError(#[from] SerdeJsonError),
    #[error("DeadPoolError {0}")]
    DeadPoolError(#[from] DeadPoolError),
    #[error("DeadpoolBuildError {0}")]
    DeadpoolBuildError(#[from] DeadpoolBuildError),
    #[error("DeadpoolConfigError {0}")]
    DeadpoolConfigError(#[from] DeadpoolConfigError),
    #[error("TokioPostgresError {0}")]
    TokioPostgresError(#[from] TokioPostgresError),
    #[error("ZipError {0}")]
    ZipError(#[from] ZipError),
    #[error("RandUniformError {0}")]
    RandUniformError(#[from] RandUniformError),
    #[error("PopenError {0}")]
    PopenError(#[from] PopenError),
    #[error("ParseIntError {0}")]
    ParseIntError(#[from] ParseIntError),
    #[error("ParseFloatError {0}")]
    ParseFloatError(#[from] ParseFloatError),
    #[error("TryFromIntError {0}")]
    TryFromIntError(#[from] TryFromIntError),
    #[error("EnvyError {0}")]
    EnvyError(#[from] EnvyError),
    #[error("UrlParseError {0}")]
    UrlParseError(#[from] UrlParseError),
    #[error("io Error {0}")]
    IoError(#[from] std::io::Error),
    #[error("tokio join error {0}")]
    JoinError(#[from] JoinError),
    #[error("Base64DecodeError {0}")]
    DecodeError(#[from] DecodeError),
    #[error("FromUtf8Error {0}")]
    FromUtf8Error(#[from] FromUtf8Error),
    #[error("Utf8Error {0}")]
    Utf8Error(#[from] Utf8Error),
    #[error("FmtError {0}")]
    FmtError(#[from] FmtError),
    #[error("PqError {0}")]
    PqError(#[from] PqError),
    #[error("PqExtractError {0}")]
    PqExtractError(#[from] PqExtractError),
    #[error("TimeParseError {0}")]
    TimeParseError(#[from] TimeParseError),
    #[error("{0}")]
    StaticCustomError(&'static str),
    #[error("{0}")]
    CustomError(StackString),
}
