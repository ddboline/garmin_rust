use failure::{err_msg, Error};
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
    Ultimate,
    Elliptical,
    Stairs,
    Lifting,
    Swimming,
    Other,
    Snowshoeing,
    Skiing,
}

impl fmt::Display for SportTypes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sport_str = match self {
            SportTypes::Running => "running".to_string(),
            SportTypes::Biking => "biking".to_string(),
            SportTypes::Walking => "walking".to_string(),
            SportTypes::Ultimate => "ultimate".to_string(),
            SportTypes::Elliptical => "elliptical".to_string(),
            SportTypes::Stairs => "stairs".to_string(),
            SportTypes::Lifting => "lifting".to_string(),
            SportTypes::Swimming => "swimming".to_string(),
            SportTypes::Other => "other".to_string(),
            SportTypes::Snowshoeing => "snowshoeing".to_string(),
            SportTypes::Skiing => "skiing".to_string(),
        };
        write!(f, "{}", sport_str)
    }
}

impl SportTypes {
    pub fn to_string(self) -> String {
        format!("{}", self)
    }
}

impl FromStr for SportTypes {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SPORT_TYPE_MAP.get(s) {
            Some(sport) => Ok(*sport),
            None => Err(err_msg(format!("Invalid Sport Type {}", s))),
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
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

pub fn convert_sport_name(sport: &str) -> Option<String> {
    let map0 = &SPORT_TYPE_MAP;

    match map0.get(sport) {
        Some(&s) => Some(s.to_string()),
        None => None,
    }
}

pub fn get_strava_activity_type(sport_type: SportTypes) -> String {
    match sport_type {
        SportTypes::Running => "run".to_string(),
        SportTypes::Biking => "ride".to_string(),
        SportTypes::Walking => "walk".to_string(),
        SportTypes::Ultimate => "ultimate".to_string(),
        SportTypes::Elliptical => "elliptical".to_string(),
        SportTypes::Stairs => "stairs".to_string(),
        SportTypes::Lifting => "lifting".to_string(),
        SportTypes::Swimming => "swim".to_string(),
        SportTypes::Other => "other".to_string(),
        SportTypes::Snowshoeing => "snowshoe".to_string(),
        SportTypes::Skiing => "nordicski".to_string(),
    }
}

pub fn convert_sport_name_to_activity_type(sport: &str) -> Option<String> {
    let map0 = &SPORT_TYPE_MAP;

    match map0.get(sport) {
        Some(&s) => Some(get_strava_activity_type(s)),
        None => None,
    }
}
