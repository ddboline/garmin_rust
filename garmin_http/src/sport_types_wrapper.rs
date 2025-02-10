use rweb::Schema;
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use stack_string::StackString;
use std::{convert::TryFrom, fmt, str::FromStr};

use garmin_lib::errors::GarminError as Error;
use garmin_utils::sport_types::SportTypes;

#[derive(Serialize, Debug, Clone, Copy, Hash, Eq, PartialEq, Schema, Deserialize)]
#[serde(into = "StackString", try_from = "StackString")]
pub enum SportTypesWrapper {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "biking")]
    Biking,
    #[serde(rename = "walking")]
    Walking,
    #[serde(rename = "hiking")]
    Hiking,
    #[serde(rename = "ultimate")]
    Ultimate,
    #[serde(rename = "elliptical")]
    Elliptical,
    #[serde(rename = "stairs")]
    Stairs,
    #[serde(rename = "lifting")]
    Lifting,
    #[serde(rename = "swimming")]
    Swimming,
    #[serde(rename = "other")]
    Other,
    #[serde(rename = "snowshoeing")]
    Snowshoeing,
    #[serde(rename = "skiing")]
    Skiing,
    #[serde(rename = "none")]
    None,
}

impl From<SportTypes> for SportTypesWrapper {
    fn from(item: SportTypes) -> Self {
        match item {
            SportTypes::Running => Self::Running,
            SportTypes::Biking => Self::Biking,
            SportTypes::Walking => Self::Walking,
            SportTypes::Hiking => Self::Hiking,
            SportTypes::Ultimate => Self::Ultimate,
            SportTypes::Elliptical => Self::Elliptical,
            SportTypes::Stairs => Self::Stairs,
            SportTypes::Lifting => Self::Lifting,
            SportTypes::Swimming => Self::Swimming,
            SportTypes::Other => Self::Other,
            SportTypes::Snowshoeing => Self::Snowshoeing,
            SportTypes::Skiing => Self::Skiing,
            SportTypes::None => Self::None,
        }
    }
}

impl From<SportTypesWrapper> for SportTypes {
    fn from(item: SportTypesWrapper) -> Self {
        match item {
            SportTypesWrapper::Running => Self::Running,
            SportTypesWrapper::Biking => Self::Biking,
            SportTypesWrapper::Walking => Self::Walking,
            SportTypesWrapper::Hiking => Self::Hiking,
            SportTypesWrapper::Ultimate => Self::Ultimate,
            SportTypesWrapper::Elliptical => Self::Elliptical,
            SportTypesWrapper::Stairs => Self::Stairs,
            SportTypesWrapper::Lifting => Self::Lifting,
            SportTypesWrapper::Swimming => Self::Swimming,
            SportTypesWrapper::Other => Self::Other,
            SportTypesWrapper::Snowshoeing => Self::Snowshoeing,
            SportTypesWrapper::Skiing => Self::Skiing,
            SportTypesWrapper::None => Self::None,
        }
    }
}

/// # Errors
/// Return error if `serialize_str` fails
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn serialize<S>(sport: &SportTypesWrapper, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let sport: SportTypes = (*sport).into();
    serializer.serialize_str(&sport.to_strava_activity())
}

/// # Errors
/// Return error if deserialization fails
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn deserialize<'de, D>(deserializer: D) -> Result<SportTypesWrapper, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    SportTypes::from_strava_activity(&s)
        .map_err(serde::de::Error::custom)
        .map(Into::into)
}

impl fmt::Display for SportTypesWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s: SportTypes = (*self).into();
        f.write_str(s.to_str())
    }
}

impl From<SportTypesWrapper> for StackString {
    fn from(item: SportTypesWrapper) -> StackString {
        let s: SportTypes = item.into();
        s.to_str().into()
    }
}

impl TryFrom<&str> for SportTypesWrapper {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        SportTypes::from_str(s).map(Into::into).map_err(Into::into)
    }
}

impl TryFrom<StackString> for SportTypesWrapper {
    type Error = Error;
    fn try_from(s: StackString) -> Result<Self, Self::Error> {
        SportTypes::from_str(s.as_str())
            .map(Into::into)
            .map_err(Into::into)
    }
}
