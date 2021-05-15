use bytes::BytesMut;
use chrono::{DateTime, Utc};
use derive_more::{Deref, Display, From, FromStr, Into};
use postgres_types::{FromSql, IsNull, ToSql};
use rweb::openapi::{Entity, Schema, Type};
use serde::{Deserialize, Serialize};

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
    Display,
    Hash,
)]
pub struct DateTimeWrapper(DateTime<Utc>);

impl Entity for DateTimeWrapper {
    #[inline]
    fn describe() -> Schema {
        Schema {
            schema_type: Some(Type::String),
            format: "datetime".into(),
            ..Schema::default()
        }
    }
}

impl<'a> FromSql<'a> for DateTimeWrapper {
    fn from_sql(
        type_: &postgres_types::Type,
        raw: &[u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let datetime = DateTime::<Utc>::from_sql(type_, raw)?;
        Ok(datetime.into())
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <DateTime<Utc> as FromSql>::accepts(ty)
    }
}

impl ToSql for DateTimeWrapper {
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
        <DateTime<Utc> as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_sql_checked(ty, out)
    }
}
