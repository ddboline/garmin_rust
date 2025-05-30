use serde::Deserialize;
use stack_string::StackString;

use garmin_lib::errors::GarminError as Error;

use crate::garmin_connect_activity::GarminConnectActivity;

const ACTIVITY_URL: &str =
    "https://connect.garmin.com/activitylist-service/activities/search/activities";
const HEARTRATE_URL: &str = "https://connect.garmin.com/wellness-service/wellness/dailyHeartRate";

#[derive(Deserialize)]
pub struct GarminConnectHarFile {
    log: GarminConnectHarLog,
}

impl GarminConnectHarFile {
    /// # Errors
    /// Return error if serde fails
    pub fn get_activities(&self) -> Result<Vec<GarminConnectActivity>, Error> {
        self.log
            .entries
            .iter()
            .find(|e| e.request.url.contains(ACTIVITY_URL))
            .and_then(|e| e.response.content.text.as_ref())
            .map_or(Ok(Vec::new()), |buf| serde_json::from_str(buf.as_str()))
            .map_err(Into::into)
    }

    #[must_use]
    pub fn get_heartrates(&self) -> Vec<&str> {
        self.log
            .entries
            .iter()
            .filter_map(|entry| {
                if entry.request.url.contains(HEARTRATE_URL) {
                    Some(entry.response.content.text.as_ref()?.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct GarminConnectHarLog {
    entries: Vec<GarminConnectEntry>,
}

#[derive(Deserialize)]
struct GarminConnectEntry {
    request: GarminConnectRequest,
    response: GarminConnectResponse,
}

#[derive(Deserialize)]
struct GarminConnectRequest {
    url: StackString,
}

#[derive(Deserialize)]
struct GarminConnectResponse {
    content: GarminConnectContent,
}

#[derive(Deserialize)]
struct GarminConnectContent {
    text: Option<StackString>,
}
