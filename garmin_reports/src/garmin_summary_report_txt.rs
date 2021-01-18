use anyhow::Error;
use chrono::{DateTime, Datelike, Utc};
use futures::future::try_join_all;
use itertools::Itertools;
use log::debug;
use postgres_query::FromSqlRow;
use stack_string::StackString;
use url::Url;

use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
        pgpool::PgPool, strava_activity::StravaActivity,
    },
    utils::{
        garmin_util::{
            days_in_month, days_in_year, print_h_m_s, METERS_PER_MILE, MONTH_NAMES, WEEKDAY_NAMES,
        },
        iso_8601_datetime::convert_datetime_to_str,
    },
};

use crate::garmin_report_options::{GarminReportAgg, GarminReportOptions};

type GarminTextEntry = (StackString, Option<StackString>);

pub trait GarminReportTrait {
    fn get_text_entry(&self) -> Result<Vec<GarminTextEntry>, Error>;
    fn get_html_entry(&self) -> Result<StackString, Error> {
        let ent = self
            .get_text_entry()?
            .into_iter()
            .map(|(s, u)| u.map_or(s, |u| u))
            .join("</td><td>");
        let cmd = self.generate_url_string();
        Ok(format!(
            "<tr><td>{}{}{}{}{}{}</td></tr>",
            r#"<button type="submit" onclick="send_command('filter="#,
            cmd,
            r#"');">"#,
            cmd,
            "</button></td><td>",
            ent.trim()
        )
        .into())
    }
    fn generate_url_string(&self) -> StackString {
        "year,running".to_string().into()
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
    pub fn get_html_entries(&self) -> Result<Vec<StackString>, Error> {
        match self {
            Self::Year(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::Month(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::Week(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::Day(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::File(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::Sport(x) => x.iter().map(GarminReportTrait::get_html_entry).collect(),
            Self::Empty => Ok(Vec::new()),
        }
    }
}

pub async fn create_report_query<T: AsRef<str>>(
    pool: &PgPool,
    options: &GarminReportOptions,
    constraints: &[T],
) -> Result<GarminReportQuery, Error> {
    let sport_constr = match options.do_sport {
        Some(x) => format!("sport = '{}'", x.to_string()),
        None => "".to_string(),
    };

    let constr = if constraints.is_empty() {
        if sport_constr.is_empty() {
            "".to_string()
        } else {
            format!("WHERE {}", sport_constr)
        }
    } else if sport_constr.is_empty() {
        format!(
            "WHERE {}",
            constraints.iter().map(AsRef::as_ref).join(" OR ")
        )
    } else {
        format!(
            "WHERE ({}) AND {}",
            constraints.iter().map(AsRef::as_ref).join(" OR "),
            sport_constr
        )
    };

    debug!("{}", constr);

    let result_vec = if let Some(agg) = &options.agg {
        match agg {
            GarminReportAgg::Year => {
                GarminReportQuery::Year(year_summary_report(&pool, &constr).await?)
            }
            GarminReportAgg::Month => {
                GarminReportQuery::Month(month_summary_report(&pool, &constr).await?)
            }
            GarminReportAgg::Week => {
                GarminReportQuery::Week(week_summary_report(&pool, &constr).await?)
            }
            GarminReportAgg::Day => {
                GarminReportQuery::Day(day_summary_report(&pool, &constr).await?)
            }
            GarminReportAgg::File => {
                GarminReportQuery::File(file_summary_report(&pool, &constr).await?)
            }
        }
    } else if options.do_sport.is_none() {
        GarminReportQuery::Sport(sport_summary_report(&pool, &constr).await?)
    } else {
        GarminReportQuery::Empty
    };

    Ok(result_vec)
}

#[derive(Debug)]
pub struct FileSummaryReport {
    datetime: DateTime<Utc>,
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
        let weekdayname = WEEKDAY_NAMES[self.isodow as usize - 1];
        let datetime = convert_datetime_to_str(self.datetime);

        debug!("{} {:?}", datetime, self);

        let mut tmp_vec = Vec::new();

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((format!("{:17}", format!("{:10}", datetime),).into(), None));
                tmp_vec.push((format!("{:02} {:3}", self.week, weekdayname).into(), None));
                if self.total_distance > 0.0 {
                    tmp_vec.push((
                        format!(
                            "{:10} {:10} {:10} {:10} {:10} {:10}",
                            self.sport,
                            format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format!("{} cal", self.total_calories),
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(self.total_duration, true)?
                        )
                        .into(),
                        None,
                    ));
                } else {
                    tmp_vec.push((
                        format!(
                            "{:10} {:10} {:10} {:10} {:10} {:10}",
                            self.sport,
                            format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format!("{} cal", self.total_calories),
                            "".to_string(),
                            "".to_string(),
                            print_h_m_s(self.total_duration, true)?
                        )
                        .into(),
                        None,
                    ));
                }
            }
            "biking" => {
                tmp_vec.push((format!("{:17}", format!("{:10}", datetime),).into(), None));
                tmp_vec.push((format!("{:02} {:3}", self.week, weekdayname).into(), None));
                tmp_vec.push((
                    format!(
                        "{:10} {:10} {:10} {:10} {:10} {:10}",
                        self.sport,
                        format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format!("{} cal", self.total_calories),
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        ),
                        "".to_string(),
                        print_h_m_s(self.total_duration, true)?
                    )
                    .into(),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format!("{:17}", format!("{:10}", datetime),).into(), None));
                tmp_vec.push((format!("{:02} {:3}", self.week, weekdayname).into(), None));
                tmp_vec.push((
                    format!(
                        "{:10} {:10} {:10} {:10} {:10} {:10}",
                        self.sport,
                        format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format!("{} cal", self.total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(self.total_duration, true)?
                    )
                    .into(),
                    None,
                ));
            }
        };
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    "\t {:7}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32)
                )
                .into(),
                None,
            ));
        } else {
            tmp_vec.push(("".into(), None));
        }
        if self.total_fitbit_steps > 0 || self.total_connect_steps > 0 {
            let fitbit_url: Option<Url> = if let Some(id) = self.fitbit_id {
                format!("https://www.fitbit.com/activities/exercise/{}", id)
                    .parse()
                    .ok()
            } else {
                None
            };
            let connect_url: Option<Url> = if let Some(id) = self.connect_id {
                format!("https://connect.garmin.com/modern/activity/{}", id)
                    .parse()
                    .ok()
            } else {
                None
            };
            let text = format!(
                " {:>16} steps",
                format!("{} / {}", self.total_fitbit_steps, self.total_connect_steps),
            )
            .into();
            let fitbit_str = if let Some(u) = fitbit_url {
                format!(
                    r#"<a href="{}" target="_blank">{}</a>"#,
                    u, self.total_fitbit_steps
                )
            } else {
                self.total_fitbit_steps.to_string()
            };
            let connect_str = if let Some(u) = connect_url {
                format!(
                    r#"<a href="{}" target="_blank">{}</a>"#,
                    u, self.total_connect_steps
                )
            } else {
                self.total_connect_steps.to_string()
            };
            let html_str =
                format!(" {:>16} steps", format!("{} / {}", fitbit_str, connect_str),).into();
            tmp_vec.push((text, Some(html_str)));
        } else {
            tmp_vec.push(("".into(), None));
        }
        if let Some(strava_title) = &self.strava_title {
            if let Some(strava_id) = &self.strava_id {
                let url: Option<Url> = format!("https://www.strava.com/activities/{}", strava_id)
                    .parse()
                    .ok();
                tmp_vec.push((
                    format!(" {}", strava_title).into(),
                    url.map(|u| {
                        format!(r#"<a href="{}" target="_blank">{}</a>"#, u, strava_title).into()
                    }),
                ));
            } else {
                tmp_vec.push((format!(" {}", strava_title).into(), None));
            }
        } else {
            tmp_vec.push(("".into(), None));
        }
        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        convert_datetime_to_str(self.datetime).into()
    }
}

async fn file_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<FileSummaryReport>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct FileSummaryReportRow {
        begin_datetime: DateTime<Utc>,
        sport: StackString,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
        summary_id: i32,
    }

    let query = format!(
        "
        SELECT begin_datetime,
                sport,
                total_calories,
                total_distance,
                total_duration,
                CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                CASE WHEN total_hr_dis > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis,
                id as summary_id
        FROM garmin_summary
        {}
        ORDER BY datetime, sport
    ",
        constr
    );

    let rows = pool.get().await?.query(query.as_str(), &[]).await?;

    let futures = rows.iter().map(|row| {
        let pool = pool.clone();
        async move {
            let item = FileSummaryReportRow::from_row(row)?;

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
                datetime: item.begin_datetime,
                week: item.begin_datetime.iso_week().week(),
                isodow: item.begin_datetime.weekday().num_days_from_monday(),
                sport: item.sport,
                total_calories: item.total_calories,
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
    week: f64,
    isodow: f64,
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
                        format!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                            self.sport,
                            format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format!("{} cal", self.total_calories),
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(self.total_duration, true)?
                        )
                        .into(),
                        None,
                    ));
                } else {
                    tmp_vec.push((
                        format!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                            self.sport,
                            format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                            format!("{} cal", self.total_calories),
                            "".to_string(),
                            "".to_string(),
                            print_h_m_s(self.total_duration, true)?
                        )
                        .into(),
                        None,
                    ));
                }
            }
            "biking" => {
                tmp_vec.push((
                    format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                        self.sport,
                        format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format!("{} cal", self.total_calories),
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        ),
                        "".to_string(),
                        print_h_m_s(self.total_duration, true)?
                    )
                    .into(),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((
                    format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", self.date, self.week, weekdayname),
                        self.sport,
                        format!("{:.2} mi", self.total_distance / METERS_PER_MILE),
                        format!("{} cal", self.total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(self.total_duration, true)?
                    )
                    .into(),
                    None,
                ));
            }
        };
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    "\t {:7}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32)
                )
                .into(),
                None,
            ));
        }
        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format!("{},file,{}", self.sport, self.date).into()
    }
}

async fn day_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<DaySummaryReport>, Error> {
    let query = format!(
        "
        WITH a AS (
            SELECT begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary
            {}
        )
        SELECT
            CAST(CAST(begin_datetime at time zone 'localtime' as date) as text) as date,
            EXTRACT(week from begin_datetime at time zone 'localtime') as week,
            EXTRACT(isodow from begin_datetime at time zone 'localtime') as isodow,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis
        FROM a
        GROUP BY sport, date, week, isodow
        ORDER BY sport, date, week, isodow
    ",
        constr
    );

    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| DaySummaryReport::from_row(row).map_err(Into::into))
        .collect()
}

#[derive(FromSqlRow, Debug)]
pub struct WeekSummaryReport {
    year: f64,
    week: f64,
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

        let mut tmp_vec = Vec::new();

        tmp_vec.push((
            format!(
                "{:15} {:7} {:10} {:10} \t",
                format!("{} week {:02}", self.year, self.week),
                self.sport,
                format!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
                format!("{} cal", self.total_calories)
            )
            .into(),
            None,
        ));

        match self.sport.as_str() {
            "running" | "walking" => {
                if self.total_distance > 0.0 {
                    tmp_vec.push((
                        format!(
                            " {:10} \t",
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            )
                        )
                        .into(),
                        None,
                    ));
                    tmp_vec.push((
                        format!(
                            " {:10} \t",
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    self.total_duration / (self.total_distance / 1000.),
                                    false
                                )?
                            )
                        )
                        .into(),
                        None,
                    ));
                } else {
                    tmp_vec.push((format!(" {:10} \t", "").into(), None));
                    tmp_vec.push((format!(" {:10} \t", "").into(), None));
                }
            }
            "biking" => {
                tmp_vec.push((
                    format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE) / (self.total_duration / 3600.)
                        )
                    )
                    .into(),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format!(" {:10} \t", "").into(), None));
            }
        }
        tmp_vec.push((
            format!(" {:10} \t", print_h_m_s(self.total_duration, true)?).into(),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    " {:7} {:2}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                )
                .into(),
                None,
            ));
        } else {
            tmp_vec.push((format!(" {:7} {:2}", "", "").into(), None));
        };
        tmp_vec.push((
            format!(
                "{:16}",
                format!("{} / {} days", self.number_of_days, total_days)
            )
            .into(),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format!("{},day,{}w{}", self.sport, self.year, self.week).into()
    }
}

async fn week_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<WeekSummaryReport>, Error> {
    let query = format!(
        "
        WITH a AS (
            SELECT begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary
            {}
        )
        SELECT
            EXTRACT(isoyear from begin_datetime at time zone 'localtime') as year,
            EXTRACT(week from begin_datetime at time zone 'localtime') as week,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM a
        GROUP BY sport, year, week
        ORDER BY sport, year, week
    ",
        constr
    );

    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| WeekSummaryReport::from_row(row).map_err(Into::into))
        .collect()
}

#[derive(FromSqlRow, Debug)]
pub struct MonthSummaryReport {
    year: f64,
    month: f64,
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

        let mut tmp_vec = Vec::new();

        tmp_vec.push((
            format!(
                "{:8} {:10} {:8} \t",
                format!("{} {}", self.year, MONTH_NAMES[self.month as usize - 1]),
                self.sport,
                format!("{:4.2} mi", (self.total_distance / METERS_PER_MILE)),
            )
            .into(),
            None,
        ));
        tmp_vec.push((
            format!("{:10} \t", format!("{} cal", self.total_calories)).into(),
            None,
        ));

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format!(
                        " {:10} \t",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ));
                tmp_vec.push((
                    format!(
                        " {:10} \t",
                        format!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ))
            }
            "biking" => {
                tmp_vec.push((
                    format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    )
                    .into(),
                    None,
                ));
            }
            _ => {
                tmp_vec.push((format!(" {:10} \t", "").into(), None));
            }
        };
        tmp_vec.push((
            format!(" {:10} \t", print_h_m_s(self.total_duration, true)?).into(),
            None,
        ));

        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    " {:7} {:2}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                )
                .into(),
                None,
            ));
        } else {
            tmp_vec.push((format!(" {:7} {:2}", " ", " ").into(), None));
        };

        tmp_vec.push((
            format!(
                "{:16}",
                format!("{} / {} days", self.number_of_days, total_days)
            )
            .into(),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format!("{},day,{:04}-{:02}", self.sport, self.year, self.month).into()
    }
}

async fn month_summary_report(
    pool: &PgPool,
    constr: &str,
) -> Result<Vec<MonthSummaryReport>, Error> {
    let query = format!(
        "
        WITH a AS (
            SELECT begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary
            {}
        )
        SELECT
            EXTRACT(year from begin_datetime at time zone 'localtime') as year,
            EXTRACT(month from begin_datetime at time zone 'localtime') as month,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM a
        GROUP BY sport, year, month
        ORDER BY sport, year, month
    ",
        constr
    );

    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| MonthSummaryReport::from_row(row).map_err(Into::into))
        .collect()
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
        let mut tmp_vec = Vec::new();

        tmp_vec.push((format!("{:10} \t", self.sport).into(), None));
        tmp_vec.push((
            format!(
                "{:10} \t",
                format!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
            )
            .into(),
            None,
        ));
        tmp_vec.push((
            format!("{:10} \t", format!("{} cal", self.total_calories)).into(),
            None,
        ));

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format!(
                        "{:10} ",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ));
                tmp_vec.push((
                    format!(
                        "{:10} ",
                        format!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ));
            }
            "biking" => {
                tmp_vec.push((
                    format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    )
                    .into(),
                    None,
                ));
            }
            _ => (),
        };

        tmp_vec.push((
            format!(" {:10} \t", print_h_m_s(self.total_duration, true)?).into(),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    " {:7} {:2}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                )
                .into(),
                None,
            ));
        } else {
            tmp_vec.push((format!(" {:7} {:2}", "", "").into(), None));
        }

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format!("year,{}", self.sport).into()
    }
}

async fn sport_summary_report(
    pool: &PgPool,
    constr: &str,
) -> Result<Vec<SportSummaryReport>, Error> {
    let query = format!(
        "
        WITH a AS (
            SELECT begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary
            {}
        )
        SELECT sport,
               sum(total_calories) as total_calories,
               sum(total_distance) as total_distance,
               sum(total_duration) as total_duration,
               sum(total_hr_dur) as total_hr_dur,
               sum(total_hr_dis) as total_hr_dis
        FROM a
        GROUP BY sport
        ORDER BY sport
        ",
        constr
    );
    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| SportSummaryReport::from_row(row).map_err(Into::into))
        .collect()
}

#[derive(FromSqlRow, Debug)]
pub struct YearSummaryReport {
    year: f64,
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

        let mut tmp_vec = Vec::new();

        tmp_vec.push((
            format!("{:5} {:10} \t", self.year, self.sport,).into(),
            None,
        ));
        tmp_vec.push((
            format!(
                "{:10} \t",
                format!("{:4.2} mi", self.total_distance / METERS_PER_MILE),
            )
            .into(),
            None,
        ));
        tmp_vec.push((
            format!("{:10} \t", format!("{} cal", self.total_calories)).into(),
            None,
        ));

        match self.sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push((
                    format!(
                        "{:10} ",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ));
                tmp_vec.push((
                    format!(
                        "{:10} ",
                        format!(
                            "{} / km",
                            print_h_m_s(
                                self.total_duration / (self.total_distance / 1000.),
                                false
                            )?
                        )
                    )
                    .into(),
                    None,
                ));
            }
            "biking" => {
                tmp_vec.push((
                    format!(
                        " {:10} ",
                        format!(
                            "{:.2} mph",
                            (self.total_distance / METERS_PER_MILE)
                                / (self.total_duration / 60. / 60.)
                        )
                    )
                    .into(),
                    None,
                ));
            }
            _ => (),
        };

        tmp_vec.push((
            format!(" {:10} \t", print_h_m_s(self.total_duration, true)?).into(),
            None,
        ));
        if self.total_hr_dur > self.total_hr_dis {
            tmp_vec.push((
                format!(
                    " {:7} {:2}",
                    format!("{} bpm", (self.total_hr_dur / self.total_hr_dis) as i32),
                    ""
                )
                .into(),
                None,
            ));
        } else {
            tmp_vec.push((format!(" {:7} {:2}", "", "").into(), None));
        }

        tmp_vec.push((
            format!(
                "{:16}",
                format!("{} / {} days", self.number_of_days, total_days)
            )
            .into(),
            None,
        ));

        Ok(tmp_vec)
    }
    fn generate_url_string(&self) -> StackString {
        format!("{},month,{}", self.sport, self.year).into()
    }
}

async fn year_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<YearSummaryReport>, Error> {
    let query = format!(
        "
        WITH a AS (
            SELECT begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dur ELSE 0.0 END AS total_hr_dur,
                   CASE WHEN total_hr_dur > 0.0 THEN total_hr_dis ELSE 0.0 END AS total_hr_dis
            FROM garmin_summary
            {}
        )
        SELECT
            EXTRACT(year from begin_datetime at time zone 'localtime') as year,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(begin_datetime at time zone 'localtime' as date)) as number_of_days
        FROM a
        GROUP BY sport, year
        ORDER BY sport, year
        ",
        constr
    );
    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| YearSummaryReport::from_row(row).map_err(Into::into))
        .collect()
}
