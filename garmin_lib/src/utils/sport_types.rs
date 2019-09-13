use failure::{format_err, Error};
use lazy_static::lazy_static;
use serde::{self, Deserialize, Deserializer, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

lazy_static! {
    static ref SPORT_TYPE_MAP: HashMap<String, SportTypes> = get_sport_type_map();
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
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
        SportTypes::None
    }
}

impl fmt::Display for SportTypes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sport_str = match self {
            SportTypes::Running => "running",
            SportTypes::Biking => "biking",
            SportTypes::Walking => "walking",
            SportTypes::Hiking => "hiking",
            SportTypes::Ultimate => "ultimate",
            SportTypes::Elliptical => "elliptical",
            SportTypes::Stairs => "stairs",
            SportTypes::Lifting => "lifting",
            SportTypes::Swimming => "swimming",
            SportTypes::Other => "other",
            SportTypes::Snowshoeing => "snowshoeing",
            SportTypes::Skiing => "skiing",
            SportTypes::None => "none",
        };
        write!(f, "{}", sport_str)
    }
}

impl SportTypes {
    pub fn to_strava_activity(self) -> String {
        match self {
            SportTypes::Running => "run",
            SportTypes::Biking => "ride",
            SportTypes::Walking => "walk",
            SportTypes::Hiking => "hike",
            SportTypes::Ultimate => "ultimate",
            SportTypes::Elliptical => "elliptical",
            SportTypes::Stairs => "stairs",
            SportTypes::Lifting => "lifting",
            SportTypes::Swimming => "swim",
            SportTypes::Other => "other",
            SportTypes::Snowshoeing => "snowshoe",
            SportTypes::Skiing => "nordicski",
            SportTypes::None => "none",
        }
        .to_string()
    }
}

impl FromStr for SportTypes {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SPORT_TYPE_MAP.get(&s.to_lowercase()) {
            Some(sport) => Ok(*sport),
            None => Err(format_err!("Invalid Sport Type {}", s)),
        }
    }
}

pub fn get_sport_type_map() -> HashMap<String, SportTypes> {
    [
        ("running", SportTypes::Running),
        ("run", SportTypes::Running),
        ("biking", SportTypes::Biking),
        ("bike", SportTypes::Biking),
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
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

pub fn convert_sport_name(sport: &str) -> Option<String> {
    sport.parse().ok().map(|s: SportTypes| s.to_string())
}

pub fn convert_sport_name_to_activity_type(sport: &str) -> Option<String> {
    sport
        .parse()
        .ok()
        .map(|s: SportTypes| s.to_strava_activity())
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn serialize<S>(sport: &SportTypes, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&sport.to_string())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<SportTypes, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}
