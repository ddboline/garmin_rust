use anyhow::Error;
use chrono::{DateTime, TimeZone, Utc};
use serde::{de, Deserialize, Deserializer, Serializer};
use stack_string::StackString;

#[must_use]
pub fn sentinel_datetime() -> DateTime<Utc> {
    Utc.ymd(0, 1, 1).and_hms(0, 0, 0)
}

#[must_use]
pub fn convert_datetime_to_str(datetime: DateTime<Utc>) -> StackString {
    StackString::from_display(datetime.format("%Y-%m-%dT%H:%M:%SZ"))
}

/// # Errors
/// Return error if `parse_from_rfc3339` fails
pub fn convert_str_to_datetime(s: &str) -> Result<DateTime<Utc>, Error> {
    DateTime::parse_from_rfc3339(&s.replace('Z', "+00:00"))
        .map(|x| x.with_timezone(&Utc))
        .map_err(Into::into)
}

/// # Errors
/// Returns error if serialization fails
pub fn serialize<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&convert_datetime_to_str(*date))
}

/// # Errors
/// Returns error if deserialization fails
pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    convert_str_to_datetime(&s).map_err(de::Error::custom)
}
