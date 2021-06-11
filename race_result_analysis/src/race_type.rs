use anyhow::{format_err, Error};
use bytes::BytesMut;
use rweb::Schema;
use serde::{Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt::{self, Display, Formatter},
    str::FromStr,
};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize, Schema)]
#[serde(into = "String", try_from = "String")]
pub enum RaceType {
    #[serde(rename = "personal")]
    Personal,
    #[serde(rename = "world_record_men")]
    WorldRecordMen,
    #[serde(rename = "world_record_women")]
    WorldRecordWomen,
}

impl From<RaceType> for String {
    fn from(item: RaceType) -> String {
        item.to_string()
    }
}

impl TryFrom<&str> for RaceType {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl TryFrom<String> for RaceType {
    type Error = Error;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(s.as_str())
    }
}

impl Display for RaceType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Personal => "personal",
            Self::WorldRecordMen => "world_record_men",
            Self::WorldRecordWomen => "world_record_women",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for RaceType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "personal" => Ok(Self::Personal),
            "world_record_men" => Ok(Self::WorldRecordMen),
            "world_record_women" => Ok(Self::WorldRecordWomen),
            _ => Err(format_err!("Invalid Race Type")),
        }
    }
}

impl<'a> FromSql<'a> for RaceType {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let s = String::from_sql(ty, raw)?.parse()?;
        Ok(s)
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

impl ToSql for RaceType {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}
