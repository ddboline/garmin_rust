use anyhow::Error;
use serde::Deserialize;
use stack_string::StackString;

use crate::strava_activity::StravaActivityHarJson;

const ACTIVITY_URL: &str = "https://www.strava.com/athlete/training_activities";

#[derive(Deserialize)]
pub struct StravaActivityHarFile {
    log: StravaActivityHarLog,
}

impl StravaActivityHarFile {
    /// # Errors
    /// Return error if serde fails
    pub fn get_activities(&self) -> Result<Option<StravaActivityHarJson>, Error> {
        self.log
            .entries
            .iter()
            .find(|e| e.request.url.contains(ACTIVITY_URL))
            .and_then(|e| e.response.content.text.as_ref())
            .map_or(Ok(None), |buf| serde_json::from_str(buf.as_str()))
            .map_err(Into::into)
    }
}

#[derive(Deserialize)]
struct StravaActivityHarLog {
    entries: Vec<StravaActivityEntry>,
}

#[derive(Deserialize)]
struct StravaActivityEntry {
    request: StravaActivityRequest,
    response: StravaActivityResponse,
}

#[derive(Deserialize)]
struct StravaActivityRequest {
    url: StackString,
}

#[derive(Deserialize)]
struct StravaActivityResponse {
    content: StravaActivityContent,
}

#[derive(Deserialize)]
struct StravaActivityContent {
    text: Option<StackString>,
}
