use chrono::{DateTime, NaiveDateTime, Utc};
use derive_more::{Display, Into};
use serde::{Deserialize, Serialize};

#[derive(Into, Copy, Clone, Serialize, Deserialize, Display)]
#[serde(from = "i64")]
pub struct GarminConnectTimestamp(DateTime<Utc>);

impl From<i64> for GarminConnectTimestamp {
    fn from(timestamp_ms: i64) -> Self {
        let timestamp: i64 = timestamp_ms / 1000;
        let datetime = DateTime::<Utc>::from_utc(
            NaiveDateTime::from_timestamp(
                timestamp,
                ((timestamp_ms - timestamp * 1000) * 1_000_000) as u32,
            ),
            Utc,
        );
        Self(datetime)
    }
}
