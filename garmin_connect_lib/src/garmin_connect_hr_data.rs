use chrono::{DateTime, Utc};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::fmt::Write;

use super::garmin_connect_timestamp::GarminConnectTimestamp;

#[derive(Serialize, Deserialize)]
pub struct GarminConnectHrData {
    #[serde(rename = "heartRateValues")]
    pub heartrate_values: Option<Vec<(GarminConnectTimestamp, Option<i32>)>>,
}

impl GarminConnectHrData {
    #[must_use]
    pub fn to_table(&self, entries: Option<usize>) -> StackString {
        if let Some(heartrate_values) = self.heartrate_values.as_ref() {
            let entries = entries.unwrap_or(heartrate_values.len());
            let rows = heartrate_values
                .iter()
                .skip(heartrate_values.len() - entries)
                .filter_map(|(timestamp, heartrate)| {
                    let datetime: DateTime<Utc> = (*timestamp).into();
                    heartrate.map(|heartrate| {
                        format_sstr!("<tr><td>{datetime}</td><td>{heartrate}</td></tr>")
                    })
                })
                .join("\n");
            format_sstr!(
                "<table border=1><thead><th>Datetime</th><th>Heart \
                 Rate</th></thead><tbody>{rows}</tbody></table>"
            )
        } else {
            "".into()
        }
    }
}
