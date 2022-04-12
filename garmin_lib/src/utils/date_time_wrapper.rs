use bytes::BytesMut;
use derive_more::{Deref, DerefMut, From, Into};
use postgres_types::{FromSql, IsNull, ToSql, Type};
use serde::{Deserialize, Serialize};
use std::fmt;
use time::{macros::datetime, OffsetDateTime};

#[derive(
    Serialize,
    Deserialize,
    Into,
    From,
    Deref,
    DerefMut,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    Debug,
    Hash,
)]
pub struct DateTimeWrapper(#[serde(with = "iso8601")] OffsetDateTime);

impl DateTimeWrapper {
    #[must_use]
    #[inline]
    pub fn from_offsetdatetime(d: OffsetDateTime) -> Self {
        Self(d)
    }

    #[must_use]
    #[inline]
    pub fn to_offsetdatetime(self) -> OffsetDateTime {
        self.0
    }

    #[must_use]
    #[inline]
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    #[must_use]
    pub fn sentinel_datetime() -> Self {
        datetime!(0000-01-01 00:00:00).assume_utc().into()
    }
}

impl fmt::Display for DateTimeWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let s = iso8601::convert_datetime_to_str(self.0);
        write!(f, "{s}")
    }
}

pub mod iso8601 {
    use anyhow::Error;
    use serde::{de, Deserialize, Deserializer, Serializer};
    use stack_string::StackString;
    use std::borrow::Cow;
    use time::{
        format_description::well_known::Rfc3339, macros::format_description, OffsetDateTime,
        UtcOffset,
    };

    #[must_use]
    pub fn convert_datetime_to_str(datetime: OffsetDateTime) -> StackString {
        datetime
            .to_offset(UtcOffset::UTC)
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
            ))
            .unwrap_or_else(|_| "".into())
            .into()
    }

    /// # Errors
    /// Return error if `parse_from_rfc3339` fails
    pub fn convert_str_to_datetime(s: &str) -> Result<OffsetDateTime, Error> {
        let s: Cow<str> = if s.contains('Z') {
            s.replace('Z', "+00:00").into()
        } else {
            s.into()
        };
        OffsetDateTime::parse(&s, &Rfc3339)
            .map(|x| x.to_offset(UtcOffset::UTC))
            .map_err(Into::into)
    }

    /// # Errors
    /// Returns error if serialization fails
    pub fn serialize<S>(date: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&convert_datetime_to_str(*date))
    }

    /// # Errors
    /// Returns error if deserialization fails
    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        convert_str_to_datetime(&s).map_err(de::Error::custom)
    }
}

impl<'a> FromSql<'a> for DateTimeWrapper {
    fn from_sql(
        type_: &Type,
        raw: &[u8],
    ) -> Result<DateTimeWrapper, Box<dyn std::error::Error + Sync + Send>> {
        OffsetDateTime::from_sql(type_, raw).map(Into::into)
    }

    fn accepts(ty: &Type) -> bool {
        <OffsetDateTime as FromSql>::accepts(ty)
    }
}

impl ToSql for DateTimeWrapper {
    fn to_sql(
        &self,
        type_: &Type,
        w: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        OffsetDateTime::to_sql(&self.0, type_, w)
    }

    fn accepts(ty: &Type) -> bool {
        <OffsetDateTime as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        OffsetDateTime::to_sql_checked(&self.0, ty, out)
    }
}
