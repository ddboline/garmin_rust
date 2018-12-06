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

pub const METERS_PER_MILE: f64 = 1609.344;
pub const MARATHON_DISTANCE_M: i32 = 42195;
pub const MARATHON_DISTANCE_MI: f64 = MARATHON_DISTANCE_M as f64 / METERS_PER_MILE;

pub const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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
    ].iter()
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
    ].iter()
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

pub fn convert_time_string(time_str: &str) -> Result<f64, Error> {
    let entries: Vec<_> = time_str.split(":").collect();
    let (h, m, s): (i32, i32, f64) = match entries.get(0) {
        Some(h) => match entries.get(1) {
            Some(m) => match entries.get(2) {
                Some(s) => (h.parse()?, m.parse()?, s.parse()?),
                None => (h.parse()?, m.parse()?, 0.),
            },
            None => (h.parse()?, 0, 0.),
        },
        None => (0, 0, 0.),
    };
    Ok(s + 60.0 * (m as f64 + 60.0 * h as f64))
}

pub fn convert_xml_local_time_to_utc(xml_local_time: &str) -> Result<String, Error> {
    let local = DateTime::parse_from_rfc3339(xml_local_time)?.with_timezone(&FixedOffset::east(0));
    Ok(local.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

pub fn get_md5sum(filename: &str) -> String {
    let command = format!("md5sum {}", filename);

    let stream = Exec::shell(command).stream_stdout().unwrap();

    let reader = BufReader::new(stream);

    for line in reader.lines() {
        for entry in line.unwrap().split_whitespace() {
            return entry.to_string();
        }
    }
    "".to_string()
}

pub fn print_h_m_s(second: f64, do_hours: bool) -> Result<String, Error> {
    let hours = (second / 3600.0) as i32;
    let minutes = (second / 60.0) as i32 - hours * 60;
    let seconds = second as i32 - minutes * 60 - hours * 3600;
    if (hours > 0) | ((hours == 0) & do_hours) {
        Ok(format!("{:02}:{:02}:{:02}", hours, minutes, seconds))
    } else if hours == 0 {
        Ok(format!("{:02}:{:02}", minutes, seconds))
    } else {
        Err(err_msg("Negative result!"))
    }
}

pub fn days_in_year(year: i32) -> i64 {
    (Utc.ymd(year + 1, 1, 1) - Utc.ymd(year, 1, 1)).num_days()
}

pub fn days_in_month(year: i32, month: u32) -> i64 {
    let mut y1_ = year;
    let mut m1_ = month + 1;
    if m1_ == 13 {
        y1_ += 1;
        m1_ = 1;
    }
    (Utc.ymd(y1_, m1_, 1) - Utc.ymd(year, month, 1)).num_days()
}

pub fn expected_calories(weight: f64, pace_min_per_mile: f64, distance: f64) -> f64 {
    let cal_per_mi = weight
        * (0.0395 + 0.00327 * (60. / pace_min_per_mile)
            + 0.000455 * (60. / pace_min_per_mile).pow(2.0)
            + 0.000801 * ((weight / 154.0) * 0.425 / weight * (60. / pace_min_per_mile).pow(3.0))
                * 60. / (60. / pace_min_per_mile));
    cal_per_mi * distance
}

pub struct PlotOpts<'a> {
    pub name: String,
    pub title: String,
    pub data: Option<&'a Vec<(f64, f64)>>,
    pub do_scatter: bool,
    pub cache_dir: String,
    pub marker: Option<String>,
    pub xlabel: String,
    pub ylabel: String,
}

impl<'a> PlotOpts<'a> {
    pub fn new() -> PlotOpts<'a> {
        PlotOpts {
            name: "".to_string(),
            title: "".to_string(),
            data: None,
            do_scatter: false,
            cache_dir: "".to_string(),
            marker: None,
            xlabel: "".to_string(),
            ylabel: "".to_string(),
        }
    }

    pub fn with_name(mut self, name: &str) -> PlotOpts<'a> {
        self.name = name.to_string();
        self
    }

    pub fn with_title(mut self, title: &str) -> PlotOpts<'a> {
        self.title = title.to_string();
        self
    }

    pub fn with_data(mut self, data: &'a Vec<(f64, f64)>) -> PlotOpts<'a> {
        self.data = Some(data);
        self
    }

    pub fn with_scatter(mut self) -> PlotOpts<'a> {
        self.do_scatter = true;
        self
    }

    pub fn with_cache_dir(mut self, cache_dir: &str) -> PlotOpts<'a> {
        self.cache_dir = cache_dir.to_string();
        self
    }

    pub fn with_marker(mut self, marker: &str) -> PlotOpts<'a> {
        self.marker = Some(marker.to_string());
        self
    }

    pub fn with_labels(mut self, xlabel: &str, ylabel: &str) -> PlotOpts<'a> {
        self.xlabel = xlabel.to_string();
        self.ylabel = ylabel.to_string();
        self
    }
}

pub fn plot_graph(opts: &PlotOpts) -> Result<String, Error> {
    let command = format!(
        "garmin-plot-graph -n {} -t {} -x {} -y {} -c {} {} {}",
        opts.name,
        format!("{}{}{}", '"', opts.title, '"'),
        format!("{}{}{}", '"', opts.xlabel, '"'),
        format!("{}{}{}", '"', opts.ylabel, '"'),
        format!("{}{}{}", '"', opts.cache_dir, '"'),
        match &opts.marker {
            Some(m) => format!("-m {0}{1}{0}", '"', m),
            None => "".to_string(),
        },
        match opts.do_scatter {
            true => "-s".to_string(),
            false => "".to_string(),
        }
    );

    debug!("{}", command);

    let input = format!("{}\n", serde_json::to_string(&opts.data)?);

    let mut popen = Exec::shell(&command)
        .stdin(Redirection::Pipe)
        .stdout(Redirection::Pipe)
        .popen()?;

    let (result, _) = popen.communicate(Some(&input))?;

    Ok(result.clone().unwrap())
}

pub fn titlecase(input: &str) -> String {
    if input.len() == 0 {
        "".to_string()
    } else {
        let firstchar = input[0..1].to_uppercase();
        format!("{}{}", firstchar, &input[1..input.len()])
    }
}
