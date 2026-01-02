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
use notify::Error as NotifyError;
use polars::error::PolarsError;
use postgres_query::{extract::Error as PqExtractError, Error as PqError};
use rand::distr::uniform::Error as RandUniformError;
use refinery::Error as RefineryError;
use reqwest::Error as ReqwestError;
use reqwest_oauth1::Error as ReqwestOauth1Error;
use roxmltree::Error as RoXmlTreeError;
use serde_json::Error as SerdeJsonError;
use serde_yaml_ng::Error as YamlError;
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
    NotifyError(Box<NotifyError>),
    #[error("RefineryError {0}")]
    RefineryError(#[from] RefineryError),
    #[error("StdoutChannelError {0}")]
    StdoutChannelError(#[from] StdoutChannelError),
    #[error("ReqwestOauth1Error {0}")]
    ReqwestOauth1Error(Box<ReqwestOauth1Error>),
    #[error("UuidError {0}")]
    UuidError(#[from] UuidError),
    #[error("GlobError {0}")]
    GlobError(#[from] GlobError),
    #[error("PatternError {0}")]
    PatternError(#[from] PatternError),
    #[error("PolarsError {0}")]
    PolarsError(Box<PolarsError>),
    #[error("TimeComponentRange {0}")]
    TimeComponentRange(Box<TimeComponentRange>),
    #[error("TelegramBotError {0}")]
    TelegramBotError(Box<TelegramBotError>),
    #[error("TimeFormatError {0}")]
    TimeFormatError(#[from] TimeFormatError),
    #[error("InvalidHeaderValue {0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),
    #[error("ReqwestError {0}")]
    ReqwestError(#[from] ReqwestError),
    #[error("RoXmlTreeError {0}")]
    RoXmlTreeError(Box<RoXmlTreeError>),
    #[error("FitParserError {0}")]
    FitParserError(#[from] FitParserError),
    #[error("SystemTimeError {0}")]
    SystemTimeError(#[from] SystemTimeError),
    #[error("AwsByteStreamError {0}")]
    AwsByteStreamError(#[from] AwsByteStreamError),
    #[error("AwsGetObjectError {0}")]
    AwsGetObjectError(Box<AwsGetObjectError>),
    #[error("AwsListObjectError {0}")]
    AwsListObjectError(Box<AwsListObjectError>),
    #[error("AwsPutObjectError {0}")]
    AwsPutObjectError(Box<AwsPutObjectError>),
    #[error("ApacheAvroError {0}")]
    ApacheAvroError(Box<ApacheAvroError>),
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
    FromUtf8Error(Box<FromUtf8Error>),
    #[error("Utf8Error {0}")]
    Utf8Error(#[from] Utf8Error),
    #[error("FmtError {0}")]
    FmtError(#[from] FmtError),
    #[error("PqError {0}")]
    PqError(Box<PqError>),
    #[error("PqExtractError {0}")]
    PqExtractError(Box<PqExtractError>),
    #[error("TimeParseError {0}")]
    TimeParseError(Box<TimeParseError>),
    #[error("{0}")]
    StaticCustomError(&'static str),
    #[error("{0}")]
    CustomError(StackString),
}

impl From<AwsGetObjectError> for GarminError {
    fn from(value: AwsGetObjectError) -> Self {
        Self::AwsGetObjectError(value.into())
    }
}

impl From<AwsListObjectError> for GarminError {
    fn from(value: AwsListObjectError) -> Self {
        Self::AwsListObjectError(value.into())
    }
}

impl From<AwsPutObjectError> for GarminError {
    fn from(value: AwsPutObjectError) -> Self {
        Self::AwsPutObjectError(value.into())
    }
}

impl From<ApacheAvroError> for GarminError {
    fn from(value: ApacheAvroError) -> Self {
        Self::ApacheAvroError(value.into())
    }
}

impl From<NotifyError> for GarminError {
    fn from(value: NotifyError) -> Self {
        Self::NotifyError(value.into())
    }
}

impl From<PqError> for GarminError {
    fn from(value: PqError) -> Self {
        Self::PqError(value.into())
    }
}

impl From<TimeComponentRange> for GarminError {
    fn from(value: TimeComponentRange) -> Self {
        Self::TimeComponentRange(value.into())
    }
}

impl From<TelegramBotError> for GarminError {
    fn from(value: TelegramBotError) -> Self {
        Self::TelegramBotError(value.into())
    }
}

impl From<RoXmlTreeError> for GarminError {
    fn from(value: RoXmlTreeError) -> Self {
        Self::RoXmlTreeError(value.into())
    }
}

impl From<PqExtractError> for GarminError {
    fn from(value: PqExtractError) -> Self {
        Self::PqExtractError(value.into())
    }
}

impl From<TimeParseError> for GarminError {
    fn from(value: TimeParseError) -> Self {
        Self::TimeParseError(value.into())
    }
}

impl From<ReqwestOauth1Error> for GarminError {
    fn from(value: ReqwestOauth1Error) -> Self {
        Self::ReqwestOauth1Error(value.into())
    }
}

impl From<PolarsError> for GarminError {
    fn from(value: PolarsError) -> Self {
        Self::PolarsError(value.into())
    }
}

impl From<FromUtf8Error> for GarminError {
    fn from(value: FromUtf8Error) -> Self {
        Self::FromUtf8Error(value.into())
    }
}

#[cfg(test)]
mod test {
    use apache_avro::Error as ApacheAvroError;
    use aws_smithy_types::byte_stream::error::Error as AwsByteStreamError;
    use base64::DecodeError;
    use deadpool_postgres::{BuildError as DeadpoolBuildError, ConfigError as DeadpoolConfigError};
    use envy::Error as EnvyError;
    use fitparser::Error as FitParserError;
    use glob::{GlobError, PatternError};
    use http::header::InvalidHeaderValue;
    use json::Error as JsonError;
    use notify::Error as NotifyError;
    use polars::error::PolarsError;
    use postgres_query::{extract::Error as PqExtractError, Error as PqError};
    use rand::distr::uniform::Error as RandUniformError;
    use refinery::Error as RefineryError;
    use reqwest::Error as ReqwestError;
    use reqwest_oauth1::Error as ReqwestOauth1Error;
    use roxmltree::Error as RoXmlTreeError;
    use serde_json::Error as SerdeJsonError;
    use serde_yaml_ng::{Error as YamlError, Error as SerdeYamlError};
    use stack_string::StackString;
    use std::{
        fmt::Error as FmtError,
        net::AddrParseError,
        num::{ParseFloatError, ParseIntError, TryFromIntError},
        str::Utf8Error,
        string::FromUtf8Error,
        time::SystemTimeError,
    };
    use stdout_channel::StdoutChannelError;
    use telegram_bot::Error as TelegramBotError;
    use time::error::{
        ComponentRange as TimeComponentRange, Format as TimeFormatError, Parse as TimeParseError,
    };
    use time_tz::system::Error as TzError;
    use tokio::task::JoinError;
    use tokio_postgres::error::Error as TokioPostgresError;
    use url::ParseError as UrlParseError;
    use uuid::Error as UuidError;
    use zip::result::ZipError;

    use crate::errors::{
        AwsGetObjectError, AwsListObjectError, AwsPutObjectError, DeadPoolError,
        GarminError as Error,
    };

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
        println!("NotifyError {}", std::mem::size_of::<NotifyError>());
        println!("AddrParseError {}", std::mem::size_of::<AddrParseError>());
        println!("SerdeYamlError {}", std::mem::size_of::<SerdeYamlError>());
        println!("SerdeJsonError {}", std::mem::size_of::<SerdeJsonError>());
        println!("PqError {}", std::mem::size_of::<PqError>());
        println!("FmtError  {}", std::mem::size_of::<FmtError>());

        println!("AddrParseError {}", std::mem::size_of::<AddrParseError>());
        println!("YamlError {}", std::mem::size_of::<YamlError>());
        println!("NotifyError {}", std::mem::size_of::<NotifyError>());
        println!("RefineryError {}", std::mem::size_of::<RefineryError>());
        println!(
            "StdoutChannelError {}",
            std::mem::size_of::<StdoutChannelError>()
        );
        println!(
            "ReqwestOauth1Error {}",
            std::mem::size_of::<ReqwestOauth1Error>()
        );
        println!("UuidError {}", std::mem::size_of::<UuidError>());
        println!("GlobError {}", std::mem::size_of::<GlobError>());
        println!("PatternError {}", std::mem::size_of::<PatternError>());
        println!("PolarsError {}", std::mem::size_of::<PolarsError>());
        println!(
            "TimeComponentRange {}",
            std::mem::size_of::<TimeComponentRange>()
        );
        println!(
            "TelegramBotError {}",
            std::mem::size_of::<TelegramBotError>()
        );
        println!("TimeFormatError {}", std::mem::size_of::<TimeFormatError>());
        println!(
            "InvalidHeaderValue {}",
            std::mem::size_of::<InvalidHeaderValue>()
        );
        println!("ReqwestError {}", std::mem::size_of::<ReqwestError>());
        println!("RoXmlTreeError {}", std::mem::size_of::<RoXmlTreeError>());
        println!("FitParserError {}", std::mem::size_of::<FitParserError>());
        println!("SystemTimeError {}", std::mem::size_of::<SystemTimeError>());
        println!(
            "AwsByteStreamError {}",
            std::mem::size_of::<AwsByteStreamError>()
        );
        println!(
            "AwsGetObjectError {}",
            std::mem::size_of::<AwsGetObjectError>()
        );
        println!(
            "AwsListObjectError {}",
            std::mem::size_of::<AwsListObjectError>()
        );
        println!(
            "AwsPutObjectError {}",
            std::mem::size_of::<AwsPutObjectError>()
        );
        println!("ApacheAvroError {}", std::mem::size_of::<ApacheAvroError>());
        println!("JsonError {}", std::mem::size_of::<JsonError>());
        println!("SerdeJsonError {}", std::mem::size_of::<SerdeJsonError>());
        println!("DeadPoolError {}", std::mem::size_of::<DeadPoolError>());
        println!(
            "DeadpoolBuildError {}",
            std::mem::size_of::<DeadpoolBuildError>()
        );
        println!(
            "DeadpoolConfigError {}",
            std::mem::size_of::<DeadpoolConfigError>()
        );
        println!(
            "TokioPostgresError {}",
            std::mem::size_of::<TokioPostgresError>()
        );
        println!("ZipError {}", std::mem::size_of::<ZipError>());
        println!(
            "RandUniformError {}",
            std::mem::size_of::<RandUniformError>()
        );
        println!("ParseIntError {}", std::mem::size_of::<ParseIntError>());
        println!("ParseFloatError {}", std::mem::size_of::<ParseFloatError>());
        println!("TryFromIntError {}", std::mem::size_of::<TryFromIntError>());
        println!("EnvyError {}", std::mem::size_of::<EnvyError>());
        println!("UrlParseError {}", std::mem::size_of::<UrlParseError>());
        println!("io Error {}", std::mem::size_of::<std::io::Error>());
        println!("tokio join error {}", std::mem::size_of::<JoinError>());
        println!("Base64DecodeError {}", std::mem::size_of::<DecodeError>());
        println!("FromUtf8Error {}", std::mem::size_of::<FromUtf8Error>());
        println!("Utf8Error {}", std::mem::size_of::<Utf8Error>());
        println!("FmtError {}", std::mem::size_of::<FmtError>());
        println!("PqError {}", std::mem::size_of::<PqError>());
        println!("PqExtractError {}", std::mem::size_of::<PqExtractError>());
        println!("TimeParseError {}", std::mem::size_of::<TimeParseError>());

        assert_eq!(std::mem::size_of::<Error>(), 40);
    }
}
