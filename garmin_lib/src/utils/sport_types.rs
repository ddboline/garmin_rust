use anyhow::{format_err, Error};
use bytes::BytesMut;
use lazy_static::lazy_static;
use rweb::Schema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use stack_string::StackString;
use std::{collections::HashMap, convert::TryFrom, fmt, str::FromStr};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

lazy_static! {
    static ref SPORT_TYPE_MAP: HashMap<StackString, SportTypes> = init_sport_type_map();
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize, Schema)]
#[serde(into = "StackString", try_from = "StackString")]
pub enum SportTypes {
    Running,
    Biking,
    Walking,
    Hiking,
    Ultimate,
    Elliptical,
    Stairs,
    Lifting,
    Swimming,
    Other,
    Snowshoeing,
    Skiing,
    None,
}

impl Default for SportTypes {
    fn default() -> Self {
        Self::None
    }
}

impl fmt::Display for SportTypes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

impl From<SportTypes> for StackString {
    fn from(item: SportTypes) -> StackString {
        item.to_string().into()
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn serialize<S>(sport: &SportTypes, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&sport.to_strava_activity())
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn deserialize<'de, D>(deserializer: D) -> Result<SportTypes, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    SportTypes::from_strava_activity(&s).map_err(serde::de::Error::custom)
}

impl SportTypes {
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Biking => "biking",
            Self::Walking => "walking",
            Self::Hiking => "hiking",
            Self::Ultimate => "ultimate",
            Self::Elliptical => "elliptical",
            Self::Stairs => "stairs",
            Self::Lifting => "lifting",
            Self::Swimming => "swimming",
            Self::Other => "other",
            Self::Snowshoeing => "snowshoeing",
            Self::Skiing => "skiing",
            Self::None => "none",
        }
    }

    pub fn to_strava_activity(self) -> StackString {
        match self {
            Self::Running => "Run",
            Self::Biking => "Ride",
            Self::Walking => "Walk",
            Self::Hiking => "Hike",
            Self::Elliptical => "Elliptical",
            Self::Stairs => "StairStepper",
            Self::Lifting => "WeightTraining",
            Self::Swimming => "Swim",
            Self::Snowshoeing => "Snowshoe",
            Self::Skiing => "NordicSki",
            Self::None => "None",
            _ => "Other",
        }
        .into()
    }

    pub fn from_strava_activity(item: &str) -> Result<Self, Error> {
        match item {
            "Run" => Ok(Self::Running),
            "Ride" => Ok(Self::Biking),
            "Walk" => Ok(Self::Walking),
            "Hike" => Ok(Self::Hiking),
            "Elliptical" => Ok(Self::Elliptical),
            "StairStepper" => Ok(Self::Stairs),
            "WeightTraining" => Ok(Self::Lifting),
            "Swim" => Ok(Self::Swimming),
            "Snowshoe" => Ok(Self::Snowshoeing),
            "NordicSki" => Ok(Self::Skiing),
            _ => Err(format_err!("Invalid activity type")),
        }
    }

    pub fn to_fitbit_activity_id(self) -> Option<u64> {
        match self {
            Self::Running => Some(90009),
            Self::Walking => Some(90013),
            Self::Biking => Some(90001),
            Self::Hiking => Some(90012),
            Self::Ultimate => Some(15250),
            Self::Elliptical => Some(90017),
            Self::Stairs => Some(12170),
            Self::Lifting => Some(2050),
            Self::Swimming => Some(18300),
            Self::Snowshoeing => Some(19190),
            Self::Skiing => Some(90015),
            _ => None,
        }
    }

    pub fn from_fitbit_activity_id(id: usize) -> Self {
        match id {
            90009 => Self::Running,
            90013 | 15000 => Self::Walking,
            90001 | 1071 => Self::Biking,
            90012 => Self::Hiking,
            15250 => Self::Ultimate,
            90017 => Self::Elliptical,
            12170 => Self::Stairs,
            2050 => Self::Lifting,
            18300 => Self::Swimming,
            19190 => Self::Snowshoeing,
            90015 => Self::Skiing,
            _ => Self::None,
        }
    }
}

impl FromStr for SportTypes {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SPORT_TYPE_MAP.get(s.to_lowercase().as_str()) {
            Some(sport) => Ok(*sport),
            None => Err(format_err!("Invalid Sport Type {}", s)),
        }
    }
}

impl TryFrom<&str> for SportTypes {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl TryFrom<StackString> for SportTypes {
    type Error = Error;
    fn try_from(s: StackString) -> Result<Self, Self::Error> {
        Self::from_str(s.as_str())
    }
}

fn init_sport_type_map() -> HashMap<StackString, SportTypes> {
    [
        ("running", SportTypes::Running),
        ("run", SportTypes::Running),
        ("bicycle", SportTypes::Biking),
        ("bicycling", SportTypes::Biking),
        ("biking", SportTypes::Biking),
        ("bike", SportTypes::Biking),
        ("cycling", SportTypes::Biking),
        ("ride", SportTypes::Biking),
        ("walking", SportTypes::Walking),
        ("walk", SportTypes::Walking),
        ("hiking", SportTypes::Hiking),
        ("hike", SportTypes::Hiking),
        ("ultimate", SportTypes::Ultimate),
        ("frisbee", SportTypes::Ultimate),
        ("elliptical", SportTypes::Elliptical),
        ("stairs", SportTypes::Stairs),
        ("lifting", SportTypes::Lifting),
        ("lift", SportTypes::Lifting),
        ("swimming", SportTypes::Swimming),
        ("swim", SportTypes::Swimming),
        ("other", SportTypes::Other),
        ("snowshoeing", SportTypes::Snowshoeing),
        ("skiing", SportTypes::Skiing),
        ("none", SportTypes::None),
    ]
    .iter()
    .map(|(k, v)| ((*k).into(), *v))
    .collect()
}

pub fn get_sport_type_map() -> &'static HashMap<StackString, SportTypes> {
    &SPORT_TYPE_MAP
}

pub fn convert_sport_name(sport: &str) -> Option<StackString> {
    sport.parse().ok().map(|s: SportTypes| s.to_string().into())
}

pub fn convert_sport_name_to_activity_type(sport: &str) -> Option<StackString> {
    sport.parse().ok().map(SportTypes::to_strava_activity)
}

impl<'a> FromSql<'a> for SportTypes {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let s = String::from_sql(ty, raw)?.parse()?;
        Ok(s)
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

impl ToSql for SportTypes {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}
