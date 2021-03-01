use chrono::{DateTime, FixedOffset, Utc};
use derive_more::Deref;
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use stack_string::StackString;

use garmin_lib::{common::garmin_config::GarminConfig, utils::sport_types::get_sport_type_map};

use crate::garmin_report_options::{GarminReportAgg, GarminReportOptions};

lazy_static! {
    static ref WEEK_REG: Regex =
        Regex::new(r"(?P<year>\d{4})w(?P<week>\d{1,2})").expect("Bad regex");
    static ref YMD_REG: Regex =
        Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})").expect("Bad regex");
    static ref YM_REG: Regex = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})").expect("Bad regex");
    static ref Y_REG: Regex = Regex::new(r"(?P<year>\d{4})").expect("Bad regex");
}

#[derive(Clone, Debug, PartialEq)]
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
                    dt.with_timezone(&Utc)
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string()
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

    fn match_pattern(config: &GarminConfig, pat: &str) -> Self {
        let gps_file = config.gps_dir.join(pat);
        if gps_file.exists() {
            Self::Filename(pat.into())
        } else if let Ok(dt) = DateTime::parse_from_rfc3339(&pat.replace("Z", "+00:00")) {
            Self::DateTime(dt)
        } else if WEEK_REG.is_match(pat) {
            let cap = WEEK_REG.captures_iter(pat).next().unwrap();
            let year = cap
                .name("year")
                .map_or_else(|| "", |s| s.as_str())
                .parse()
                .expect("Unexpected behavior");
            let week = cap
                .name("week")
                .map_or_else(|| "", |s| s.as_str())
                .parse()
                .expect("Unexpected behavior");
            Self::IsoWeek { year, week }
        } else if YMD_REG.is_match(pat) {
            let cap = YMD_REG.captures_iter(pat).next().unwrap();
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
            Self::YearMonthDay { year, month, day }
        } else if YM_REG.is_match(pat) {
            let cap = YM_REG.captures_iter(pat).next().unwrap();
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
            Self::YearMonth { year, month }
        } else if Y_REG.is_match(pat) {
            let cap = Y_REG.captures_iter(pat).next().unwrap();
            let year = cap
                .name("year")
                .map_or_else(|| "", |s| s.as_str())
                .parse()
                .expect("Unexpected behvior");
            Self::Year(year)
        } else {
            Self::Query(pat.into())
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
            .map(GarminConstraint::to_query_string)
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
                        self.constraints
                            .push(GarminConstraint::match_pattern(config, pat));
                    }
                }
            };
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::DateTime;

    use garmin_lib::common::garmin_config::GarminConfig;

    use crate::garmin_constraints::GarminConstraint;

    #[test]
    fn test_garmin_constraints() -> Result<(), Error> {
        let dt = DateTime::parse_from_rfc3339("2019-02-09T13:06:13+00:00")?;
        let cs = GarminConstraint::DateTime(dt);
        let obs = cs.to_query_string();
        println!("{}", obs);
        let exp = "replace(to_char(a.begin_datetime at time zone 'utc', \
                   'YYYY-MM-DD%HH24:MI:SSZ'), '%', 'T') = '2019-02-09T13:06:13Z'";
        assert_eq!(obs, exp);
        Ok(())
    }

    #[test]
    fn test_patterns() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let result = GarminConstraint::match_pattern(&config, "2014w12");
        assert_eq!(
            result,
            GarminConstraint::IsoWeek {
                year: 2014,
                week: 12
            }
        );
        let result = GarminConstraint::match_pattern(&config, "2014w1");
        assert_eq!(
            result,
            GarminConstraint::IsoWeek {
                year: 2014,
                week: 1
            }
        );
        let result = GarminConstraint::match_pattern(&config, "2020-12");
        assert_eq!(
            result,
            GarminConstraint::YearMonth {
                year: 2020,
                month: 12
            }
        );
        let result = GarminConstraint::match_pattern(&config, "Manitou");
        assert_eq!(result, GarminConstraint::Query("Manitou".into()));
        let result = GarminConstraint::match_pattern(&config, "2001-12-05T01:23:45Z");
        let expected = "2001-12-05T01:23:45Z".parse()?;
        assert_eq!(result, GarminConstraint::DateTime(expected));
        Ok(())
    }
}
