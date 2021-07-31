use anyhow::Error;
use chrono::{DateTime, TimeZone, Utc};
use serde::{de, Deserialize, Deserializer, Serializer};

use super::datetime_wrapper::DateTimeWrapper;

pub fn sentinel_datetime() -> DateTimeWrapper {
    Utc.ymd(0, 1, 1).and_hms(0, 0, 0).into()
}

pub fn convert_datetime_to_str(datetime: DateTimeWrapper) -> String {
    datetime.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn convert_str_to_datetime(s: &str) -> Result<DateTimeWrapper, Error> {
    DateTime::parse_from_rfc3339(&s.replace("Z", "+00:00"))
        .map(|x| x.with_timezone(&Utc).into())
        .map_err(Into::into)
}

pub fn serialize<S>(date: &DateTimeWrapper, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&convert_datetime_to_str(*date))
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTimeWrapper, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    convert_str_to_datetime(&s).map_err(de::Error::custom)
}
