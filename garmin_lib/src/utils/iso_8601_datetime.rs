use anyhow::Error;
use serde::{de, Deserialize, Deserializer, Serializer};
use stack_string::StackString;
use time::{
    format_description::well_known::Rfc3339,
    macros::{datetime, format_description},
    OffsetDateTime,
};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};

#[must_use]
pub fn sentinel_datetime() -> OffsetDateTime {
    datetime!(0000-01-01 00:00:00).assume_utc()
}

#[must_use]
pub fn convert_datetime_to_str(datetime: OffsetDateTime) -> StackString {
    datetime
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
        ))
        .unwrap_or_else(|_| "".into())
        .into()
}

/// # Errors
/// Return error if `parse_from_rfc3339` fails
pub fn convert_str_to_datetime(s: &str) -> Result<OffsetDateTime, Error> {
    OffsetDateTime::parse(&s.replace('Z', "+00:00"), &Rfc3339)
        .map(|x| x.to_timezone(UTC))
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
