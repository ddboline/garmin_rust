use anyhow::Error;
use chrono::{DateTime, Utc};
use log::debug;
use postgres_query::FromSqlRow;

use crate::common::pgpool::PgPool;
use crate::reports::garmin_report_options::{GarminReportAgg, GarminReportOptions};
use crate::utils::garmin_util::{
    days_in_month, days_in_year, print_h_m_s, METERS_PER_MILE, MONTH_NAMES, WEEKDAY_NAMES,
};
use crate::utils::iso_8601_datetime::convert_datetime_to_str;

pub async fn create_report_query(
    pool: &PgPool,
    options: &GarminReportOptions,
    constraints: &[String],
) -> Result<Vec<Vec<String>>, Error> {
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
        format!("WHERE {}", constraints.join(" OR "))
    } else {
        format!("WHERE ({}) AND {}", constraints.join(" OR "), sport_constr)
    };

    debug!("{}", constr);

    let result_vec = if let Some(agg) = &options.agg {
        match agg {
            GarminReportAgg::Year => year_summary_report(&pool, &constr).await?,
            GarminReportAgg::Month => month_summary_report(&pool, &constr).await?,
            GarminReportAgg::Week => week_summary_report(&pool, &constr).await?,
            GarminReportAgg::Day => day_summary_report(&pool, &constr).await?,
            GarminReportAgg::File => file_summary_report(&pool, &constr).await?,
        }
    } else if options.do_sport.is_none() {
        sport_summary_report(&pool, &constr).await?
    } else {
        Vec::new()
    };

    Ok(result_vec)
}

async fn file_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct FileSummaryReport {
        datetime: DateTime<Utc>,
        week: f64,
        isodow: f64,
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
    }

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
            begin_datetime as datetime,
            EXTRACT(week from begin_datetime at time zone 'localtime') as week,
            EXTRACT(isodow from begin_datetime at time zone 'localtime') as isodow,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis
        FROM a
        GROUP BY sport, datetime, week, isodow
        ORDER BY sport, datetime, week, isodow
    ",
        constr
    );

    debug!("{}", query);

    pool.get()
        .await?
        .query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| {
            let row = FileSummaryReport::from_row(row)?;

            let weekdayname = WEEKDAY_NAMES[row.isodow as usize - 1];
            let datetime = convert_datetime_to_str(row.datetime);

            debug!("{} {:?}", datetime, row);

            let mut tmp_vec = Vec::new();

            match row.sport.as_str() {
                "running" | "walking" => {
                    if row.total_distance > 0.0 {
                        tmp_vec.push(format!(
                            "{:27} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:20} {:02} {:3}", datetime, row.week, weekdayname),
                            row.sport,
                            format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                            format!("{} cal", row.total_calories),
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(row.total_duration, true)?
                        ));
                    } else {
                        tmp_vec.push(format!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:10} {:02} {:3}", datetime, row.week, weekdayname),
                            row.sport,
                            format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                            format!("{} cal", row.total_calories),
                            "".to_string(),
                            "".to_string(),
                            print_h_m_s(row.total_duration, true)?
                        ));
                    }
                }
                "biking" => {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", datetime, row.week, weekdayname),
                        row.sport,
                        format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                        format!("{} cal", row.total_calories),
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE) / (row.total_duration / 3600.)
                        ),
                        "".to_string(),
                        print_h_m_s(row.total_duration, true)?
                    ));
                }
                _ => {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", datetime, row.week, weekdayname),
                        row.sport,
                        format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                        format!("{} cal", row.total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(row.total_duration, true)?
                    ));
                }
            };
            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    "\t {:7}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32)
                ));
            }
            Ok(tmp_vec)
        })
        .collect()
}

async fn day_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct DaySummaryReport {
        date: String,
        week: f64,
        isodow: f64,
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
    }

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
        .map(|row| {
            let row = DaySummaryReport::from_row(row)?;

            let weekdayname = WEEKDAY_NAMES[row.isodow as usize - 1];

            debug!("{:?}", row);

            let mut tmp_vec = Vec::new();

            match row.sport.as_str() {
                "running" | "walking" => {
                    if row.total_distance > 0.0 {
                        tmp_vec.push(format!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:10} {:02} {:3}", row.date, row.week, weekdayname),
                            row.sport,
                            format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                            format!("{} cal", row.total_calories),
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            ),
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / 1000.),
                                    false
                                )?
                            ),
                            print_h_m_s(row.total_duration, true)?
                        ));
                    } else {
                        tmp_vec.push(format!(
                            "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                            format!("{:10} {:02} {:3}", row.date, row.week, weekdayname),
                            row.sport,
                            format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                            format!("{} cal", row.total_calories),
                            "".to_string(),
                            "".to_string(),
                            print_h_m_s(row.total_duration, true)?
                        ));
                    }
                }
                "biking" => {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", row.date, row.week, weekdayname),
                        row.sport,
                        format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                        format!("{} cal", row.total_calories),
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE) / (row.total_duration / 3600.)
                        ),
                        "".to_string(),
                        print_h_m_s(row.total_duration, true)?
                    ));
                }
                _ => {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", row.date, row.week, weekdayname),
                        row.sport,
                        format!("{:.2} mi", row.total_distance / METERS_PER_MILE),
                        format!("{} cal", row.total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(row.total_duration, true)?
                    ));
                }
            };
            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    "\t {:7}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32)
                ));
            }
            Ok(tmp_vec)
        })
        .collect()
}

async fn week_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct WeekSummaryReport {
        year: f64,
        week: f64,
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
        number_of_days: i64,
    }

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
        .map(|row| {
            let row = WeekSummaryReport::from_row(row)?;

            let total_days = 7;

            debug!("{:?}", row);

            let mut tmp_vec = Vec::new();

            tmp_vec.push(format!(
                "{:15} {:7} {:10} {:10} \t",
                format!("{} week {:02}", row.year, row.week),
                row.sport,
                format!("{:4.2} mi", row.total_distance / METERS_PER_MILE),
                format!("{} cal", row.total_calories)
            ));

            match row.sport.as_str() {
                "running" | "walking" => {
                    if row.total_distance > 0.0 {
                        tmp_vec.push(format!(
                            " {:10} \t",
                            format!(
                                "{} / mi",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / METERS_PER_MILE),
                                    false
                                )?
                            )
                        ));
                        tmp_vec.push(format!(
                            " {:10} \t",
                            format!(
                                "{} / km",
                                print_h_m_s(
                                    row.total_duration / (row.total_distance / 1000.),
                                    false
                                )?
                            )
                        ));
                    } else {
                        tmp_vec.push(format!(" {:10} \t", ""));
                        tmp_vec.push(format!(" {:10} \t", ""));
                    }
                }
                "biking" => {
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE) / (row.total_duration / 3600.)
                        )
                    ));
                }
                _ => {
                    tmp_vec.push(format!(" {:10} \t", ""));
                }
            }
            tmp_vec.push(format!(" {:10} \t", print_h_m_s(row.total_duration, true)?));
            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    " {:7} {:2}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32),
                    ""
                ));
            } else {
                tmp_vec.push(format!(" {:7} {:2}", "", ""));
            };
            tmp_vec.push(format!(
                "{:16}",
                format!("{} / {} days", row.number_of_days, total_days)
            ));

            Ok(tmp_vec)
        })
        .collect()
}

async fn month_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct MonthSummaryReport {
        year: f64,
        month: f64,
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
        number_of_days: i64,
    }

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
        .map(|row| {
            let row = MonthSummaryReport::from_row(row)?;

            let total_days = days_in_month(row.year as i32, row.month as u32);

            debug!("{:?}", row);

            let mut tmp_vec = Vec::new();

            tmp_vec.push(format!(
                "{:8} {:10} {:8} \t",
                format!("{} {}", row.year, MONTH_NAMES[row.month as usize - 1]),
                row.sport,
                format!("{:4.2} mi", (row.total_distance / METERS_PER_MILE)),
            ));
            tmp_vec.push(format!("{:10} \t", format!("{} cal", row.total_calories)));

            match row.sport.as_str() {
                "running" | "walking" => {
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                row.total_duration / (row.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ));
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{} / km",
                            print_h_m_s(row.total_duration / (row.total_distance / 1000.), false)?
                        )
                    ))
                }
                "biking" => {
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE)
                                / (row.total_duration / 60. / 60.)
                        )
                    ));
                }
                _ => {
                    tmp_vec.push(format!(" {:10} \t", ""));
                }
            };
            tmp_vec.push(format!(" {:10} \t", print_h_m_s(row.total_duration, true)?));

            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    " {:7} {:2}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32),
                    ""
                ));
            } else {
                tmp_vec.push(format!(" {:7} {:2}", " ", " "));
            };

            tmp_vec.push(format!(
                "{:16}",
                format!("{} / {} days", row.number_of_days, total_days)
            ));

            Ok(tmp_vec)
        })
        .collect()
}

async fn sport_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct SportSummaryReport {
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
    }

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
        .map(|row| {
            let row = SportSummaryReport::from_row(row)?;

            debug!("{:?}", row);
            let mut tmp_vec = Vec::new();

            tmp_vec.push(format!("{:10} \t", row.sport));
            tmp_vec.push(format!(
                "{:10} \t",
                format!("{:4.2} mi", row.total_distance / METERS_PER_MILE),
            ));
            tmp_vec.push(format!("{:10} \t", format!("{} cal", row.total_calories)));

            match row.sport.as_str() {
                "running" | "walking" => {
                    tmp_vec.push(format!(
                        "{:10} ",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                row.total_duration / (row.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ));
                    tmp_vec.push(format!(
                        "{:10} ",
                        format!(
                            "{} / km",
                            print_h_m_s(row.total_duration / (row.total_distance / 1000.), false)?
                        )
                    ));
                }
                "biking" => {
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE)
                                / (row.total_duration / 60. / 60.)
                        )
                    ));
                }
                _ => (),
            };

            tmp_vec.push(format!(" {:10} \t", print_h_m_s(row.total_duration, true)?));
            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    " {:7} {:2}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32),
                    ""
                ));
            } else {
                tmp_vec.push(format!(" {:7} {:2}", "", ""));
            }

            Ok(tmp_vec)
        })
        .collect()
}

async fn year_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<Vec<String>>, Error> {
    #[derive(FromSqlRow, Debug)]
    struct YearSummaryReport {
        year: f64,
        sport: String,
        total_calories: i64,
        total_distance: f64,
        total_duration: f64,
        total_hr_dur: f64,
        total_hr_dis: f64,
        number_of_days: i64,
    }

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
        .map(|row| {
            let row = YearSummaryReport::from_row(row)?;

            let total_days = days_in_year(row.year as i32);

            debug!("{:?}", row);

            let mut tmp_vec = Vec::new();

            tmp_vec.push(format!("{:5} {:10} \t", row.year, row.sport,));
            tmp_vec.push(format!(
                "{:10} \t",
                format!("{:4.2} mi", row.total_distance / METERS_PER_MILE),
            ));
            tmp_vec.push(format!("{:10} \t", format!("{} cal", row.total_calories)));

            match row.sport.as_str() {
                "running" | "walking" => {
                    tmp_vec.push(format!(
                        "{:10} ",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                row.total_duration / (row.total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ));
                    tmp_vec.push(format!(
                        "{:10} ",
                        format!(
                            "{} / km",
                            print_h_m_s(row.total_duration / (row.total_distance / 1000.), false)?
                        )
                    ));
                }
                "biking" => {
                    tmp_vec.push(format!(
                        " {:10} ",
                        format!(
                            "{:.2} mph",
                            (row.total_distance / METERS_PER_MILE)
                                / (row.total_duration / 60. / 60.)
                        )
                    ));
                }
                _ => (),
            };

            tmp_vec.push(format!(" {:10} \t", print_h_m_s(row.total_duration, true)?));
            if row.total_hr_dur > row.total_hr_dis {
                tmp_vec.push(format!(
                    " {:7} {:2}",
                    format!("{} bpm", (row.total_hr_dur / row.total_hr_dis) as i32),
                    ""
                ));
            } else {
                tmp_vec.push(format!(" {:7} {:2}", "", ""));
            }

            tmp_vec.push(format!(
                "{:16}",
                format!("{} / {} days", row.number_of_days, total_days)
            ));

            Ok(tmp_vec)
        })
        .collect()
}
