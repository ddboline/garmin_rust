use chrono::{DateTime, FixedOffset};
use derive_more::Deref;
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use stack_string::StackString;

use garmin_lib::{common::garmin_config::GarminConfig, utils::sport_types::get_sport_type_map};

use crate::garmin_report_options::{GarminReportAgg, GarminReportOptions};

lazy_static! {
    static ref YMD_REG: Regex =
        Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})").expect("Bad regex");
    static ref YM_REG: Regex = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})").expect("Bad regex");
    static ref Y_REG: Regex = Regex::new(r"(?P<year>\d{4})").expect("Bad regex");
}

#[derive(Clone, Debug)]
pub enum GarminConstraint {
    Latest,
    IsoWeek { year: i32, week: u8 },
    Filename(StackString),
    DateTime(DateTime<FixedOffset>),
    YearMonthDay { year: i32, month: u8, day: u8 },
    YearMonth { year: i32, month: u8 },
    Year(i32),
    Query(StackString),
}

impl Default for GarminConstraint {
    fn default() -> Self {
        Self::Latest
    }
}

impl GarminConstraint {
    fn to_query_string(&self) -> String {
        match self {
            Self::Latest => {
                "a.begin_datetime=(select max(begin_datetime) from garmin_summary)".into()
            }
            Self::IsoWeek { year, week } => {
                format!(
                    "(EXTRACT(isoyear from a.begin_datetime at time zone 'localtime') = {} AND
                      EXTRACT(week from a.begin_datetime at time zone 'localtime') = {})",
                    year, week
                )
            }
            Self::Filename(filename) => format!("filename = '{}'", filename),
            Self::DateTime(dt) => {
                format!(
                    "replace({}, '%', 'T') = '{}'",
                    "to_char(a.begin_datetime at time zone 'utc', 'YYYY-MM-DD%HH24:MI:SSZ')",
                    dt.to_rfc3339()
                )
            }
            Self::YearMonthDay { year, month, day } => {
                format!(
                    "replace({}, '%', 'T') like '{:04}-{:02}-{:02}T%'",
                    "to_char(a.begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                    year,
                    month,
                    day
                )
            }
            Self::YearMonth { year, month } => {
                format!(
                    "replace({}, '%', 'T') like '{:04}-{:02}-%'",
                    "to_char(a.begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                    year,
                    month
                )
            }
            Self::Year(year) => {
                format!(
                    "replace({}, '%', 'T') like '{:04}-%'",
                    "to_char(a.begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                    year
                )
            }
            Self::Query(query) => {
                format!("lower(b.name) like '%{}%'", query.to_lowercase())
            }
        }
    }
}

#[derive(Default, Debug, Deref)]
pub struct GarminConstraints {
    pub constraints: Vec<GarminConstraint>,
}

impl GarminConstraints {
    pub fn to_query_string(&self) -> String {
        self.constraints
            .iter()
            .map(|x| x.to_query_string())
            .join(" OR ")
    }

    pub fn process_pattern<T, U>(
        &mut self,
        config: &GarminConfig,
        patterns: T,
    ) -> GarminReportOptions
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let mut options = GarminReportOptions::new();

        let sport_type_map = get_sport_type_map();

        for pattern in patterns {
            match pattern.as_ref() {
                "year" => options.agg = Some(GarminReportAgg::Year),
                "month" => options.agg = Some(GarminReportAgg::Month),
                "week" => options.agg = Some(GarminReportAgg::Week),
                "day" => options.agg = Some(GarminReportAgg::Day),
                "file" => options.agg = Some(GarminReportAgg::File),
                "sport" => options.do_sport = None,
                "latest" => self.constraints.push(GarminConstraint::default()),
                pat => {
                    if let Some(x) = sport_type_map.get(pat) {
                        options.do_sport = Some(*x)
                    } else {
                        self.match_patterns(config, pat);
                    }
                }
            };
        }

        options
    }

    fn match_patterns(&mut self, config: &GarminConfig, pat: &str) {
        if pat.contains('w') {
            let vals: Vec<_> = pat.split('w').collect();
            if vals.len() >= 2 {
                if let Ok(year) = vals[0].parse::<i32>() {
                    if let Ok(week) = vals[1].parse::<u8>() {
                        self.constraints
                            .push(GarminConstraint::IsoWeek { year, week });
                    }
                }
            }
        } else if pat.starts_with("q=") {
            self.constraints
                .push(GarminConstraint::Query(pat[2..].into()));
        } else {
            let gps_file = config.gps_dir.join(pat);
            if gps_file.exists() {
                self.constraints
                    .push(GarminConstraint::Filename(pat.into()));
            } else if let Ok(dt) = DateTime::parse_from_rfc3339(&pat.replace("Z", "+00:00")) {
                self.constraints.push(GarminConstraint::DateTime(dt));
            } else if YMD_REG.is_match(pat) {
                for cap in YMD_REG.captures_iter(pat) {
                    let year = cap
                        .name("year")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    let month = cap
                        .name("month")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    let day = cap
                        .name("day")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    self.constraints
                        .push(GarminConstraint::YearMonthDay { year, month, day });
                }
            } else if YM_REG.is_match(pat) {
                for cap in YM_REG.captures_iter(pat) {
                    let year = cap
                        .name("year")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    let month = cap
                        .name("month")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    self.constraints
                        .push(GarminConstraint::YearMonth { year, month });
                }
            } else if Y_REG.is_match(pat) {
                for cap in Y_REG.captures_iter(pat) {
                    let year = cap
                        .name("year")
                        .map_or_else(|| "", |s| s.as_str())
                        .parse()
                        .expect("Unexpected behvior");
                    self.constraints.push(GarminConstraint::Year(year));
                }
            }
        }
    }
}
