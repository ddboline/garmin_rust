use derive_more::{Display, Into};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

#[derive(Into, Copy, Clone, Serialize, Deserialize, Display)]
#[serde(from = "i64")]
pub struct GarminConnectTimestamp(OffsetDateTime);

impl From<i64> for GarminConnectTimestamp {
    fn from(timestamp_ms: i64) -> Self {
        let timestamp: i64 = timestamp_ms / 1000;
        let datetime = OffsetDateTime::from_unix_timestamp(timestamp).unwrap()
            + Duration::milliseconds(timestamp_ms - timestamp * 1000);
        Self(datetime)
    }
}
