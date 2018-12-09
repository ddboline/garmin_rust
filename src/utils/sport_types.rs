extern crate chrono;
extern crate num;
extern crate serde_json;

use num::traits::Pow;

use std::io::BufRead;
use std::io::BufReader;
use subprocess::{Exec, Redirection};

use chrono::prelude::*;

use failure::{err_msg, Error};
use std::collections::HashMap;


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
    .map(|(k, v)| (k.to_string(), v.clone()))
    .collect()
}

pub fn get_sport_type_string_map() -> HashMap<SportTypes, String> {
    [
        (SportTypes::Running, "running"),
        (SportTypes::Biking, "biking"),
        (SportTypes::Walking, "walking"),
        (SportTypes::Ultimate, "ultimate"),
        (SportTypes::Elliptical, "elliptical"),
        (SportTypes::Stairs, "stairs"),
        (SportTypes::Lifting, "lifting"),
        (SportTypes::Swimming, "swimming"),
        (SportTypes::Other, "other"),
        (SportTypes::Snowshoeing, "snowshoeing"),
        (SportTypes::Skiing, "skiing"),
    ]
    .iter()
    .map(|(k, v)| (k.clone(), v.to_string()))
    .collect()
}

pub fn convert_sport_name(sport: &str) -> Option<String> {
    let map0 = get_sport_type_map();
    let map1 = get_sport_type_string_map();

    match map0.get(sport) {
        Some(&s) => Some(map1.get(&s).unwrap().clone()),
        None => None,
    }
}
