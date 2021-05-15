use bytes::BytesMut;
use chrono::{NaiveDate, NaiveTime};
use derive_more::{Deref, Display, From, FromStr, Into};
use postgres_types::{FromSql, IsNull, ToSql};
use rweb::openapi::{Entity, Schema, Type};
use serde::{Deserialize, Serialize};

#[derive(
    Serialize,
    Deserialize,
    Debug,
    Display,
    FromStr,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    Deref,
    Into,
    From,
)]
pub struct NaiveDateWrapper(NaiveDate);

impl Entity for NaiveDateWrapper {
    #[inline]
    fn describe() -> Schema {
        Schema {
            schema_type: Some(Type::String),
            format: "naivedate".into(),
            ..Schema::default()
        }
    }
}

impl<'a> FromSql<'a> for NaiveDateWrapper {
    fn from_sql(
        type_: &postgres_types::Type,
        raw: &[u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let date = NaiveDate::from_sql(type_, raw)?;
        Ok(date.into())
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <NaiveDate as FromSql>::accepts(ty)
    }
}

impl ToSql for NaiveDateWrapper {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        ToSql::to_sql(&self.0, ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <NaiveDate as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_sql_checked(ty, out)
    }
}

#[derive(
    Serialize,
    Deserialize,
    Debug,
    FromStr,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    Deref,
    Into,
    From,
)]
pub struct NaiveTimeWrapper(NaiveTime);

impl Entity for NaiveTimeWrapper {
    #[inline]
    fn describe() -> Schema {
        Schema {
            schema_type: Some(Type::String),
            format: "naivetime".into(),
            ..Schema::default()
        }
    }
}

impl<'a> FromSql<'a> for NaiveTimeWrapper {
    fn from_sql(
        type_: &postgres_types::Type,
        raw: &[u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let date = NaiveTime::from_sql(type_, raw)?;
        Ok(date.into())
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <NaiveTime as FromSql>::accepts(ty)
    }
}

impl ToSql for NaiveTimeWrapper {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        ToSql::to_sql(&self.0, ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <NaiveTime as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_sql_checked(ty, out)
    }
}
