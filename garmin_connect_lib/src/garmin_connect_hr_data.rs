use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use super::garmin_connect_timestamp::GarminConnectTimestamp;

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(GarminConnectTimestamp, Option<i32>)>>,
}

impl GarminConnectHrData {
    pub fn to_table(&self) -> StackString {
        if let Some(heartrate_values) = self.heartrate_values.as_ref() {
            let rows: Vec<_> = heartrate_values
                .iter()
                .filter_map(|(timestamp, heartrate)| {
                    let datetime: DateTime<Utc> = (*timestamp).into();
                    heartrate.map(|heartrate| {
                        format!(
                            "<tr><td>{datetime}</td><td>{heartrate}</td></tr>",
                            datetime = datetime,
                            heartrate = heartrate
                        )
                    })
                })
                .collect();
            format!(
                "<table border=1><thead><th>Datetime</th><th>Heart \
                 Rate</th></thead><tbody>{}</tbody></table>",
                rows.join("\n")
            )
            .into()
        } else {
            "".into()
        }
    }
}
