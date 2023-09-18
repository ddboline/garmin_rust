#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

pub mod fitbit_archive;
pub mod fitbit_client;
pub mod fitbit_heartrate;
pub mod fitbit_statistics_summary;
pub mod scale_measurement;

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

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(GarminConnectTimestamp, Option<i32>)>>,
}
