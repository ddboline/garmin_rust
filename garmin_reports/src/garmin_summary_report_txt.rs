use anyhow::Error;
use futures::future::try_join_all;
use log::debug;
use postgres_query::{query_dyn, FromSqlRow};
use stack_string::{format_sstr, StackString};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
        pgpool::PgPool, strava_activity::StravaActivity,
    },
    utils::{
        date_time_wrapper::iso8601::convert_datetime_to_str,
        garmin_util::{
            days_in_month, days_in_year, print_h_m_s, METERS_PER_MILE, MONTH_NAMES, WEEKDAY_NAMES,
        },
    },
};

use crate::{
    garmin_constraints::GarminConstraints,
    garmin_report_options::{GarminReportAgg, GarminReportOptions},
};

pub struct HtmlResult {
    pub text: Option<StackString>,
    pub url: Option<Url>,
}

type GarminTextEntry = (StackString, Option<HtmlResult>);

pub trait GarminReportTrait {
    /// # Errors
    /// Returns error if getting text entry fails
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error>;

    #[must_use]
    fn generate_url_string(&self) -> StackString {
        "year,running".into()
    }
}

pub enum GarminReportQuery {
    Year(Vec<YearSummaryReport>),
    Month(Vec<MonthSummaryReport>),
    Week(Vec<WeekSummaryReport>),
    Day(Vec<DaySummaryReport>),
    File(Vec<FileSummaryReport>),
    Sport(Vec<SportSummaryReport>),
    Empty,
}

impl GarminReportQuery {
    /// # Errors
    /// Return error if `get_text_entry` fails
    pub fn get_text_entries(&self) -> Result<Vec<Vec<GarminTextEntry>>, Error> {
        match self {
            Self::Year(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::Month(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::Week(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::Day(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::File(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::Sport(x) => x.iter().map(GarminReportTrait::get_text_entry).collect(),
            Self::Empty => Ok(Vec::new()),
        }
    }
}

/// # Errors
/// Return error if db queries fail
pub async fn create_report_query(
    pool: &PgPool,
    options: &GarminReportOptions,
    constraints: &GarminConstraints,
) -> Result<GarminReportQuery, Error> {
    let sport_constr = if let Some(x) = options.do_sport {
        format_sstr!("sport = '{x}'")
    } else {
        StackString::new()
    };

    let mut constr = StackString::new();
    if constraints.is_empty() {
        if !sport_constr.is_empty() {
            constr = format_sstr!("WHERE {sport_constr}");
        }
    } else if sport_constr.is_empty() {
        constr = format_sstr!("WHERE {}", constraints.to_query_string());
    } else {
        constr = format_sstr!(
            "WHERE ({}) AND {}",
            constraints.to_query_string(),
            sport_constr
        );
    }

    debug!("{}", constr);

    let result_vec = if let Some(agg) = &options.agg {
        match agg {
            GarminReportAgg::Year => {
                GarminReportQuery::Year(year_summary_report(pool, &constr).await?)
            }
            GarminReportAgg::Month => {
                GarminReportQuery::Month(month_summary_report(pool, &constr).await?)
            }
            GarminReportAgg::Week => {
                GarminReportQuery::Week(week_summary_report(pool, &constr).await?)
            }
            GarminReportAgg::Day => {
                GarminReportQuery::Day(day_summary_report(pool, &constr).await?)
            }
            GarminReportAgg::File => {
                GarminReportQuery::File(file_summary_report(pool, &constr).await?)
            }
        }
    } else if options.do_sport.is_none() {
        GarminReportQuery::Sport(sport_summary_report(pool, &constr).await?)
    } else {
        GarminReportQuery::Empty
    };

    Ok(result_vec)
}

#[derive(Debug)]
pub struct FileSummaryReport {
    datetime: OffsetDateTime,
    week: u32,
    isodow: u32,
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
    total_fitbit_steps: i64,
    fitbit_id: Option<i64>,
    total_connect_steps: i64,
    connect_id: Option<i64>,
    strava_title: Option<StackString>,
    strava_id: Option<i64>,
}

impl GarminReportTrait for FileSummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        let weekdayname = WEEKDAY_NAMES[self.isodow as usize];
        let datetime = convert_datetime_to_str(self.datetime);

        debug!("{} {:?}", datetime, self);

        let mut tmp_vec = Vec::new();

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((format_sstr!("{:17}", format_sstr!("{datetime:10}"),), None));
                tmp_vec.push((format_sstr!("{:02} {:3}", self.week, weekdayname), None));
                if self.total_distance > 0.0 {
                    tmp_vec.push((
                        format_sstr!(
                            "{:10} {:10} {:10} {:10} {:10} {:10}",
                            self.sport,
                            format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format_sstr!("{} cal", self.total_calories),
                            format_sstr!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format_sstr!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(self.total_duration, true)?
                        ),
                        None,
                    ));
                } else {
                    tmp_vec.push((
                        format_sstr!(
                            "{:10} {:10} {:10} {:10} {:10} {:10}",
                            self.sport,
                            format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format_sstr!("{} cal", self.total_calories),
                            "",
                            "",
                            print_h_m_s(self.total_duration, true)?
                        ),
                        None,
                    ));
                }
            }
            "biking" => {
                tmp_vec.push((format_sstr!("{:17}", format_sstr!("{datetime:10}"),), None));
                tmp_vec.push((format_sstr!("{:02} {:3}", self.week, weekdayname), None));
                tmp_vec.push((
                    format_sstr!(
                        "{:10} {:10} {:10} {:10} {:10} {:10}",
                        self.sport,
                        format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format_sstr!("{} cal", self.total_calories),
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        ),
                        "",
                        print_h_m_s(self.total_duration, true)?
                    ),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format_sstr!("{:17}", format_sstr!("{datetime:10}"),), None));
                tmp_vec.push((format_sstr!("{:02} {:3}", self.week, weekdayname), None));
                tmp_vec.push((
                    format_sstr!(
                        "{:10} {:10} {:10} {:10} {:10} {:10}",
                        self.sport,
                        format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format_sstr!("{} cal", self.total_calories),
                        "",
                        "",
                        print_h_m_s(self.total_duration, true)?
                    ),
                    None,
                ));
            }
        };
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    "\t {:7}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32)
                ),
                None,
            ));
        } else {
            tmp_vec.push(("".into(), None));
        }
        if self.total_fitbit_steps > 0 || self.total_connect_steps > 0 {
            let fitbit_url: Option<Url> = if let Some(id) = self.fitbit_id {
                format_sstr!("https://www.fitbit.com/activities/exercise/{id}")
                    .parse()
                    .ok()
            } else {
                None
            };
            let connect_url: Option<Url> = if let Some(id) = self.connect_id {
                format_sstr!("https://connect.garmin.com/modern/activity/{id}")
                    .parse()
                    .ok()
            } else {
                None
            };
            let text = format_sstr!(
                " {:>16} steps",
                format_sstr!("{} / {}", self.total_fitbit_steps, self.total_connect_steps),
            );
            let fitbit_str = if let Some(u) = fitbit_url {
                format_sstr!(
                    r#"<a href="{}" target="_blank">{}</a>"#,
                    u,
                    self.total_fitbit_steps
                )
            } else {
                format_sstr!("{}", self.total_fitbit_steps)
            };
            let connect_str = if let Some(u) = connect_url {
                format_sstr!(
                    r#"<a href="{}" target="_blank">{}</a>"#,
                    u,
                    self.total_connect_steps
                )
            } else {
                format_sstr!("{}", self.total_connect_steps)
            };
            let html_str = format_sstr!(
                " {:>16} steps",
                format_sstr!("{fitbit_str} / {connect_str}"),
            );
            tmp_vec.push((
                text,
                Some(HtmlResult {
                    text: Some(html_str),
                    url: None,
                }),
            ));
        } else {
            tmp_vec.push(("".into(), None));
        }
        if let Some(strava_title) = &self.strava_title {
            if let Some(strava_id) = &self.strava_id {
                let url: Option<Url> =
                    format_sstr!("https://www.strava.com/activities/{strava_id}")
                        .parse()
                        .ok();
                tmp_vec.push((
                    format_sstr!(" {strava_title}"),
                    url.map(|u| HtmlResult {
                        text: Some(format_sstr!("{strava_title}")),
                        url: Some(u),
                    }),
                ));
            } else {
                tmp_vec.push((format_sstr!(" {strava_title}"), None));
            }
        } else {
            tmp_vec.push(("".into(), None));
        }
        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        convert_datetime_to_str(self.datetime)
    }
}

async fn file_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<FileSummaryReport>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct FileSummaryReportRow {
        datetime: OffsetDateTime,
        sport: StackString,
        total_calories: i32,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
        summary_id: Uuid,
    }

    let query = format_sstr!(
        "
        SELECT a.begin_datetime as datetime,
                a.sport,
                a.total_calories,
                a.total_distance,
                a.total_duration,
                CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                CASE WHEN a.total_hr_dis > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis,
                a.id as summary_id
        FROM garmin_summary a
        LEFT JOIN strava_activities b ON a.id = b.summary_id
        {constr}
        ORDER BY datetime, sport
    "
    );
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    let items: Vec<FileSummaryReportRow> = query.fetch(&conn).await?;

    let futures = items.into_iter().map(|item| {
        let pool = pool.clone();
        async move {
            let strava_activity =
                StravaActivity::get_from_summary_id(&pool, item.summary_id).await?;
            let strava_title = strava_activity.as_ref().map(|s| s.name.clone());
            let strava_id = strava_activity.as_ref().map(|s| s.id);

            let fitbit_activity =
                FitbitActivity::get_from_summary_id(&pool, item.summary_id).await?;
            let total_fitbit_steps = fitbit_activity.as_ref().and_then(|a| a.steps).unwrap_or(0);
            let fitbit_id = fitbit_activity.as_ref().map(|a| a.log_id);

            let connect_activity =
                GarminConnectActivity::get_from_summary_id(&pool, item.summary_id).await?;
            let total_connect_steps = connect_activity.as_ref().and_then(|a| a.steps).unwrap_or(0);
            let connect_id = connect_activity.as_ref().map(|a| a.activity_id);

            let result = FileSummaryReport {
                datetime: item.datetime,
                week: u32::from(item.datetime.iso_week()),
                isodow: u32::from(item.datetime.weekday().number_days_from_monday()),
                sport: item.sport,
                total_calories: i64::from(item.total_calories),
                total_distance: item.total_distance,
                total_duration: item.total_duration,
                total_hr_dur: item.total_hr_dur,
                total_hr_dis: item.total_hr_dis,
                total_fitbit_steps,
                fitbit_id,
                total_connect_steps,
                connect_id,
                strava_title,
                strava_id,
            };
            Ok(result)
        }
    });
    try_join_all(futures).await
}

#[derive(FromSqlRow, Debug)]
pub struct DaySummaryReport {
    date: StackString,
    week: i32,
    isodow: i32,
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
}

impl GarminReportTrait for DaySummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        let weekdayname = WEEKDAY_NAMES[self.isodow as usize - 1];

        debug!("{:?}", self);

        let mut tmp_vec = Vec::new();

        match self.sport.as_str() {
            "running" | "walking" => {
                if self.total_distance > 0.0 {
                    tmp_vec.push((
                        format_sstr!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format_sstr!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                            self.sport,
                            format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format_sstr!("{} cal", self.total_calories),
                            format_sstr!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format_sstr!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(self.total_duration, true)?
                        ),
                        None,
                    ));
                } else {
                    tmp_vec.push((
                        format_sstr!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format_sstr!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                            self.sport,
                            format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format_sstr!("{} cal", self.total_calories),
                            "",
                            "",
                            print_h_m_s(self.total_duration, true)?
                        ),
                        None,
                    ));
                }
            }
            "biking" => {
                tmp_vec.push((
                    format_sstr!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format_sstr!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                        self.sport,
                        format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format_sstr!("{} cal", self.total_calories),
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        ),
                        "",
                        print_h_m_s(self.total_duration, true)?
                    ),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((
                    format_sstr!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format_sstr!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                        self.sport,
                        format_sstr!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format_sstr!("{} cal", self.total_calories),
                        "",
                        "",
                        print_h_m_s(self.total_duration, true)?
                    ),
                    None,
                ));
            }
        };
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    "\t {:7}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32)
                ),
                None,
            ));
        }
        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format_sstr!("{},file,{}", self.sport, self.date)
    }
}

async fn day_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<DaySummaryReport>, Error> {
    let query = format_sstr!(
        "
        WITH c AS (
            SELECT a.begin_datetime,
                   a.sport,
                   a.total_calories,
                   a.total_distance,
                   a.total_duration,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        )
        SELECT
            CAST(CAST(begin_datetime at time zone 'localtime' as date) as text) as date,
            CAST(EXTRACT(week from begin_datetime at time zone 'localtime') AS INT) as week,
            CAST(EXTRACT(isodow from begin_datetime at time zone 'localtime') AS INT) as isodow,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis
        FROM c
        GROUP BY sport, date, week, isodow
        ORDER BY sport, date, week, isodow
    "
    );
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

#[derive(FromSqlRow, Debug)]
pub struct WeekSummaryReport {
    year: i32,
    week: i32,
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
    number_of_days: i64,
}

impl GarminReportTrait for WeekSummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        let total_days = 7;

        debug!("{:?}", self);

        let mut tmp_vec = vec![(
            format_sstr!(
                "{:15} {:7} {:10} {:10} \t",
                format_sstr!("{} week {:02}", self.year, self.week),
                self.sport,
                format_sstr!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
                format_sstr!("{} cal", self.total_calories)
            ),
            None,
        )];

        match self.sport.as_str() {
            "running" | "walking" => {
                if self.total_distance > 0.0 {
                    tmp_vec.push((
                        format_sstr!(
                            " {:10} \t",
                            format_sstr!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            )
                        ),
                        None,
                    ));
                    tmp_vec.push((
                        format_sstr!(
                            " {:10} \t",
                            format_sstr!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            )
                        ),
                        None,
                    ));
                } else {
                    tmp_vec.push((format_sstr!(" {:10} \t", ""), None));
                    tmp_vec.push((format_sstr!(" {:10} \t", ""), None));
                }
            }
            "biking" => {
                tmp_vec.push((
                    format_sstr!(
                        " {:10} \t",
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        )
                    ),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format_sstr!(" {:10} \t", ""), None));
            }
        }
        tmp_vec.push((
            format_sstr!(" {:10} \t", print_h_m_s(self.total_duration, true)?),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    " {:7} {:2}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                ),
                None,
            ));
        } else {
            tmp_vec.push((format_sstr!(" {:7} {:2}", "", ""), None));
        };
        tmp_vec.push((
            format_sstr!(
                "{:16}",
                format_sstr!("{} / {} days", self.number_of_days, total_days)
            ),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format_sstr!("{},day,{}w{}", self.sport, self.year, self.week)
    }
}

async fn week_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<WeekSummaryReport>, Error> {
    let query = format_sstr!(
        "
        WITH c AS (
            SELECT a.begin_datetime,
                   a.sport,
                   a.total_calories,
                   a.total_distance,
                   a.total_duration,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        )
        SELECT
            CAST(EXTRACT(isoyear from begin_datetime at time zone 'localtime') AS INT) as year,
            CAST(EXTRACT(week from begin_datetime at time zone 'localtime') AS INT) as week,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM c
        GROUP BY sport, year, week
        ORDER BY sport, year, week
    "
    );
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

#[derive(FromSqlRow, Debug)]
pub struct MonthSummaryReport {
    year: i32,
    month: i32,
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
    number_of_days: i64,
}

impl GarminReportTrait for MonthSummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        let total_days = days_in_month(self.year as i32, self.month as u32);

        debug!("{:?}", self);

        let mut tmp_vec = vec![(
            format_sstr!(
                "{:8} {:10} {:8} \t",
                format_sstr!("{} {}", self.year, MONTH_NAMES[self.month as usize - 1]),
                self.sport,
                format_sstr!("{:4.2} mi", (self.total_distance / METERS_PER_MILE)),
            ),
            None,
        )];
        tmp_vec.push((
            format_sstr!("{:10} \t", format_sstr!("{} cal", self.total_calories)),
            None,
        ));

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format_sstr!(
                        " {:10} \t",
                        format_sstr!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ),
                    None,
                ));
                tmp_vec.push((
                    format_sstr!(
                        " {:10} \t",
                        format_sstr!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    ),
                    None,
                ));
            }
            "biking" => {
                tmp_vec.push((
                    format_sstr!(
                        " {:10} \t",
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    ),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format_sstr!(" {:10} \t", ""), None));
            }
        };
        tmp_vec.push((
            format_sstr!(" {:10} \t", print_h_m_s(self.total_duration, true)?),
            None,
        ));

        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    " {:7} {:2}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                ),
                None,
            ));
        } else {
            tmp_vec.push((format_sstr!(" {:7} {:2}", " ", " "), None));
        };

        tmp_vec.push((
            format_sstr!(
                "{:16}",
                format_sstr!("{} / {} days", self.number_of_days, total_days)
            ),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format_sstr!("{},day,{:04}-{:02}", self.sport, self.year, self.month)
    }
}

async fn month_summary_report(
    pool: &PgPool,
    constr: &str,
) -> Result<Vec<MonthSummaryReport>, Error> {
    let query = format_sstr!(
        "
        WITH c AS (
            SELECT a.begin_datetime,
                   a.sport,
                   a.total_calories,
                   a.total_distance,
                   a.total_duration,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        )
        SELECT
            CAST(EXTRACT(year from begin_datetime at time zone 'localtime') AS INT) as year,
            CAST(EXTRACT(month from begin_datetime at time zone 'localtime') AS INT) as month,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM c
        GROUP BY sport, year, month
        ORDER BY sport, year, month
    "
    );
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

#[derive(FromSqlRow, Debug)]
pub struct SportSummaryReport {
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
}

impl GarminReportTrait for SportSummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        debug!("{:?}", self);
        let mut tmp_vec = vec![
            (format_sstr!("{:10} \t", self.sport), None),
            (
                format_sstr!(
                    "{:10} \t",
                    format_sstr!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
                ),
                None,
            ),
            (
                format_sstr!("{:10} \t", format_sstr!("{} cal", self.total_calories)),
                None,
            ),
        ];

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format_sstr!(
                        "{:10} ",
                        format_sstr!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ),
                    None,
                ));
                tmp_vec.push((
                    format_sstr!(
                        "{:10} ",
                        format_sstr!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    ),
                    None,
                ));
            }
            "biking" => {
                tmp_vec.push((
                    format_sstr!(
                        " {:10} \t",
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    ),
                    None,
                ));
            }
            _ => (),
        };

        tmp_vec.push((
            format_sstr!(" {:10} \t", print_h_m_s(self.total_duration, true)?),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    " {:7} {:2}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                ),
                None,
            ));
        } else {
            tmp_vec.push((format_sstr!(" {:7} {:2}", "", ""), None));
        }

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format_sstr!("year,{}", self.sport)
    }
}

async fn sport_summary_report(
    pool: &PgPool,
    constr: &str,
) -> Result<Vec<SportSummaryReport>, Error> {
    let query = format_sstr!(
        "
        WITH c AS (
            SELECT a.begin_datetime,
                   a.sport,
                   a.total_calories,
                   a.total_distance,
                   a.total_duration,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        )
        SELECT sport,
               sum(total_calories) as total_calories,
               sum(total_distance) as total_distance,
               sum(total_duration) as total_duration,
               sum(total_hr_dur) as total_hr_dur,
               sum(total_hr_dis) as total_hr_dis
        FROM c
        GROUP BY sport
        ORDER BY sport
        "
    );
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

#[derive(FromSqlRow, Debug)]
pub struct YearSummaryReport {
    year: i32,
    sport: StackString,
    total_calories: i64,
    total_distance: f64,
    total_duration: f64,
    total_hr_dur: f64,
    total_hr_dis: f64,
    number_of_days: i64,
}

impl GarminReportTrait for YearSummaryReport {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error> {
        let total_days = days_in_year(self.year as i32);

        debug!("{:?}", self);

        let mut tmp_vec = vec![
            (format_sstr!("{:5} {:10} \t", self.year, self.sport,), None),
            (
                format_sstr!(
                    "{:10} \t",
                    format_sstr!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
                ),
                None,
            ),
            (
                format_sstr!("{:10} \t", format_sstr!("{} cal", self.total_calories)),
                None,
            ),
        ];

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format_sstr!(
                        "{:10} ",
                        format_sstr!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ),
                    None,
                ));
                tmp_vec.push((
                    format_sstr!(
                        "{:10} ",
                        format_sstr!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    ),
                    None,
                ));
            }
            "biking" => {
                tmp_vec.push((
                    format_sstr!(
                        " {:10} ",
                        format_sstr!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    ),
                    None,
                ));
            }
            _ => (),
        };

        tmp_vec.push((
            format_sstr!(" {:10} \t", print_h_m_s(self.total_duration, true)?),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format_sstr!(
                    " {:7} {:2}",
                    format_sstr!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                ),
                None,
            ));
        } else {
            tmp_vec.push((format_sstr!(" {:7} {:2}", "", ""), None));
        }

        tmp_vec.push((
            format_sstr!(
                "{:16}",
                format_sstr!("{} / {} days", self.number_of_days, total_days)
            ),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format_sstr!("{},month,{}", self.sport, self.year)
    }
}

async fn year_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<YearSummaryReport>, Error> {
    let query = format_sstr!(
        "
        WITH c AS (
            SELECT a.begin_datetime,
                   a.sport,
                   a.total_calories,
                   a.total_distance,
                   a.total_duration,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN a.total_hr_dur > 0.0 THEN a.total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        )
        SELECT
            CAST(EXTRACT(year from begin_datetime at time zone 'localtime') AS INT) as year,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM c
        GROUP BY sport, year
        ORDER BY sport, year
        "
    );
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}
