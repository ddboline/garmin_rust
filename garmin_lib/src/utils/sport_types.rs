use anyhow::{format_err, Error};
use bytes::BytesMut;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::TryFrom, fmt, str::FromStr};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

lazy_static! {
    static ref SPORT_TYPE_MAP: HashMap<String, SportTypes> = init_sport_typ_map();
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
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
        let sport_str = match self {
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
        };
        write!(f, "{}", sport_str)
    }
}

impl From<SportTypes> for String {
    fn from(item: SportTypes) -> String {
        item.to_string()
    }
}

impl SportTypes {
    pub fn to_strava_activity(self) -> String {
        match self {
            Self::Running => "run",
            Self::Biking => "ride",
            Self::Walking => "walk",
            Self::Hiking => "hike",
            Self::Ultimate => "ultimate",
            Self::Elliptical => "elliptical",
            Self::Stairs => "stairs",
            Self::Lifting => "lifting",
            Self::Swimming => "swim",
            Self::Other => "other",
            Self::Snowshoeing => "snowshoe",
            Self::Skiing => "nordicski",
            Self::None => "none",
        }
        .to_string()
    }

    pub fn to_fitbit_activity(self) -> Option<String> {
        match self {
            Self::Running => Some("Running".into()),
            Self::Biking => Some("Bicycling".into()),
            Self::Walking => Some("Walking".into()),
            _ => None,
        }
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

impl TryFrom<&str> for SportTypes {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl TryFrom<String> for SportTypes {
    type Error = Error;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

fn init_sport_typ_map() -> HashMap<String, SportTypes> {
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
    .map(|(k, v)| ((*k).to_string(), *v))
    .collect()
}

pub fn get_sport_type_map() -> &'static HashMap<String, SportTypes> {
    &SPORT_TYPE_MAP
}

pub fn convert_sport_name(sport: &str) -> Option<String> {
    sport.parse().ok().map(|s: SportTypes| s.to_string())
}

pub fn convert_sport_name_to_activity_type(sport: &str) -> Option<String> {
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
