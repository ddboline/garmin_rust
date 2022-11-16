use serde::{Deserialize, Serialize};

use super::garmin_connect_timestamp::GarminConnectTimestamp;

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(GarminConnectTimestamp, Option<i32>)>>,
}
