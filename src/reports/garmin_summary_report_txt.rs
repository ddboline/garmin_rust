extern crate rayon;

use failure::Error;

use crate::reports::garmin_report_options::GarminReportOptions;
use crate::utils::garmin_util::{
    days_in_month, days_in_year, print_h_m_s, PgPool, METERS_PER_MILE, MONTH_NAMES, WEEKDAY_NAMES,
};

pub fn create_report_query(
    pool: &PgPool,
    options: &GarminReportOptions,
    constraints: &[String],
) -> Result<Vec<String>, Error> {
    let sport_constr = match options.do_sport {
        Some(x) => format!("sport = '{}'", x.to_string()),
        None => "".to_string(),
    };

    let constr = match constraints.len() {
        0 => match sport_constr.len() {
            0 => "".to_string(),
            _ => format!("WHERE {}", sport_constr),
        },
        _ => match sport_constr.len() {
            0 => format!("WHERE {}", constraints.join(" OR ")),
            _ => format!("WHERE ({}) AND {}", constraints.join(" OR "), sport_constr),
        },
    };

    debug!("{}", constr);

    let result_vec = if options.do_all_sports {
        sport_summary_report(&pool, &constr)?
    } else if options.do_year {
        year_summary_report(&pool, &constr)?
    } else if options.do_month {
        month_summary_report(&pool, &constr)?
    } else if options.do_week {
        week_summary_report(&pool, &constr)?
    } else if options.do_day {
        day_summary_report(&pool, &constr)?
    } else if options.do_file {
        file_summary_report(&pool, &constr)?
    } else {
        vec!["".to_string()]
    };

    Ok(result_vec)
}

fn file_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();
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
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            EXTRACT(isodow from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as isodow,
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

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let datetime: String = row.get(0);
        let week: f64 = row.get(1);
        let dow: f64 = row.get(2);
        let sport: String = row.get(3);
        let total_calories: i64 = row.get(4);
        let total_distance: f64 = row.get(5);
        let total_duration: f64 = row.get(6);
        let total_hr_dur: f64 = row.get(7);
        let total_hr_dis: f64 = row.get(8);

        let weekdayname = WEEKDAY_NAMES[dow as usize - 1];

        debug!(
            "{} {} {} {} {} {} {} {} {}",
            datetime,
            week,
            dow,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
        );

        let mut tmp_vec = Vec::new();

        match sport.as_str() {
            "running" | "walking" => {
                if total_distance > 0.0 {
                    tmp_vec.push(format!(
                        "{:27} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:20} {:02} {:3}", datetime, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                total_duration / (total_distance / METERS_PER_MILE),
                                false
                            )?
                        ),
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false)?
                        ),
                        print_h_m_s(total_duration, true)?
                    ));
                } else {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", datetime, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(total_duration, true)?
                    ));
                }
            }
            "biking" => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", datetime, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    format!(
                        "{:.2} mph",
                        (total_distance / METERS_PER_MILE) / (total_duration / 3600.)
                    ),
                    "".to_string(),
                    print_h_m_s(total_duration, true)?
                ));
            }
            _ => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", datetime, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    "".to_string(),
                    "".to_string(),
                    print_h_m_s(total_duration, true)?
                ));
            }
        };
        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                "\t {:7}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32)
            ));
        }
        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}

fn day_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();
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
            CAST(CAST(CAST(begin_datetime as timestamp with time zone) at time zone 'EST' as date) as text) as date,
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            EXTRACT(isodow from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as isodow,
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

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let date: String = row.get(0);
        let week: f64 = row.get(1);
        let dow: f64 = row.get(2);
        let sport: String = row.get(3);
        let total_calories: i64 = row.get(4);
        let total_distance: f64 = row.get(5);
        let total_duration: f64 = row.get(6);
        let total_hr_dur: f64 = row.get(7);
        let total_hr_dis: f64 = row.get(8);

        let weekdayname = WEEKDAY_NAMES[dow as usize - 1];

        debug!(
            "{} {} {} {} {} {} {} {} {}",
            date,
            week,
            dow,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
        );

        let mut tmp_vec = Vec::new();

        match sport.as_str() {
            "running" | "walking" => {
                if total_distance > 0.0 {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", date, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                total_duration / (total_distance / METERS_PER_MILE),
                                false
                            )?
                        ),
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false)?
                        ),
                        print_h_m_s(total_duration, true)?
                    ));
                } else {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", date, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        "".to_string(),
                        "".to_string(),
                        print_h_m_s(total_duration, true)?
                    ));
                }
            }
            "biking" => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", date, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    format!(
                        "{:.2} mph",
                        (total_distance / METERS_PER_MILE) / (total_duration / 3600.)
                    ),
                    "".to_string(),
                    print_h_m_s(total_duration, true)?
                ));
            }
            _ => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", date, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    "".to_string(),
                    "".to_string(),
                    print_h_m_s(total_duration, true)?
                ));
            }
        };
        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                "\t {:7}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32)
            ));
        }
        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}

fn week_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();
    let query = format!("
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
            EXTRACT(isoyear from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM a
        GROUP BY sport, year, week
        ORDER BY sport, year, week
    ", constr);

    debug!("{}", query);

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let year: f64 = row.get(0);
        let week: f64 = row.get(1);
        let sport: String = row.get(2);
        let total_calories: i64 = row.get(3);
        let total_distance: f64 = row.get(4);
        let total_duration: f64 = row.get(5);
        let total_hr_dur: f64 = row.get(6);
        let total_hr_dis: f64 = row.get(7);
        let number_of_days: i64 = row.get(8);

        let total_days = 7;

        debug!(
            "{} {} {} {} {} {} {} {} {}",
            year,
            week,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
            number_of_days
        );

        let mut tmp_vec = Vec::new();

        tmp_vec.push(format!(
            "{:15} {:7} {:10} {:10} \t",
            format!("{} week {:02}", year, week),
            sport,
            format!("{:4.2} mi", total_distance / METERS_PER_MILE),
            format!("{} cal", total_calories)
        ));

        match sport.as_str() {
            "running" | "walking" => {
                if total_distance > 0.0 {
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{} / mi",
                            print_h_m_s(
                                total_duration / (total_distance / METERS_PER_MILE),
                                false
                            )?
                        )
                    ));
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false)?
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
                        (total_distance / METERS_PER_MILE) / (total_duration / 3600.)
                    )
                ));
            }
            _ => {
                tmp_vec.push(format!(" {:10} \t", ""));
            }
        }
        tmp_vec.push(format!(" {:10} \t", print_h_m_s(total_duration, true)?));
        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                " {:7} {:2}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32),
                ""
            ));
        } else {
            tmp_vec.push(format!(" {:7} {:2}", "", ""));
        };
        tmp_vec.push(format!(
            "{:16}",
            format!("{} / {} days", number_of_days, total_days)
        ));

        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}

fn month_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();
    let query = format!("
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
            EXTRACT(year from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            EXTRACT(month from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as month,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM a
        GROUP BY sport, year, month
        ORDER BY sport, year, month
    ", constr);

    debug!("{}", query);

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let year: f64 = row.get(0);
        let month: f64 = row.get(1);
        let sport: String = row.get(2);
        let total_calories: i64 = row.get(3);
        let total_distance: f64 = row.get(4);
        let total_duration: f64 = row.get(5);
        let total_hr_dur: f64 = row.get(6);
        let total_hr_dis: f64 = row.get(7);
        let number_of_days: i64 = row.get(8);

        let total_days = days_in_month(year as i32, month as u32);

        debug!(
            "{} {} {} {} {} {} {} {} {}",
            year,
            month,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
            number_of_days
        );

        let mut tmp_vec = Vec::new();

        tmp_vec.push(format!(
            "{:8} {:10} {:8} \t {:10} \t",
            format!("{} {}", year, MONTH_NAMES[month as usize - 1]),
            sport,
            format!("{:4.2} mi", (total_distance / METERS_PER_MILE)),
            format!("{} cal", total_calories)
        ));

        match sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{} / mi",
                        print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)?
                    )
                ));
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{} / km",
                        print_h_m_s(total_duration / (total_distance / 1000.), false)?
                    )
                ))
            }
            "biking" => {
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{:.2} mph",
                        (total_distance / METERS_PER_MILE) / (total_duration / 60. / 60.)
                    )
                ));
            }
            _ => {
                tmp_vec.push(format!(" {:10} \t", ""));
            }
        };
        tmp_vec.push(format!(" {:10} \t", print_h_m_s(total_duration, true)?));

        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                " {:7} {:2}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32),
                ""
            ));
        } else {
            tmp_vec.push(format!(" {:7} {:2}", " ", " "));
        };

        tmp_vec.push(format!(
            "{:16}",
            format!("{} / {} days", number_of_days, total_days)
        ));

        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}

fn sport_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();

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

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let sport: String = row.get(0);
        let total_calories: i64 = row.get(1);
        let total_distance: f64 = row.get(2);
        let total_duration: f64 = row.get(3);
        let total_hr_dur: f64 = row.get(4);
        let total_hr_dis: f64 = row.get(5);

        debug!(
            "{} {} {} {} {} {}",
            sport, total_calories, total_distance, total_duration, total_hr_dur, total_hr_dis
        );
        let mut tmp_vec = Vec::new();

        tmp_vec.push(format!(
            "{:10} \t {:10} \t {:10} \t",
            sport,
            format!("{:4.2} mi", total_distance / METERS_PER_MILE),
            format!("{} cal", total_calories)
        ));

        match sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push(format!(
                    "{:10} ",
                    format!(
                        "{} / mi",
                        print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)?
                    )
                ));
                tmp_vec.push(format!(
                    "{:10} ",
                    format!(
                        "{} / km",
                        print_h_m_s(total_duration / (total_distance / 1000.), false)?
                    )
                ));
            }
            "biking" => {
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{:.2} mph",
                        (total_distance / METERS_PER_MILE) / (total_duration / 60. / 60.)
                    )
                ));
            }
            _ => (),
        };

        tmp_vec.push(format!(" {:10} \t", print_h_m_s(total_duration, true)?));
        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                " {:7} {:2}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32),
                ""
            ));
        } else {
            tmp_vec.push(format!(" {:7} {:2}", "", ""));
        }

        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}

fn year_summary_report(pool: &PgPool, constr: &str) -> Result<Vec<String>, Error> {
    let mut result_vec = Vec::new();

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
            EXTRACT(year from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM a
        GROUP BY sport, year
        ORDER BY sport, year
        ",
        constr
    );
    debug!("{}", query);

    let conn = pool.get()?;

    for row in conn.query(&query, &[])?.iter() {
        let year: f64 = row.get(0);
        let sport: String = row.get(1);
        let total_calories: i64 = row.get(2);
        let total_distance: f64 = row.get(3);
        let total_duration: f64 = row.get(4);
        let total_hr_dur: f64 = row.get(5);
        let total_hr_dis: f64 = row.get(6);
        let number_of_days: i64 = row.get(7);

        let total_days = days_in_year(year as i32);

        debug!(
            "{} {} {} {} {} {} {} {}",
            year,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
            number_of_days
        );

        let mut tmp_vec = Vec::new();

        tmp_vec.push(format!(
            "{:5} {:10} \t {:10} \t {:10} \t",
            year,
            sport,
            format!("{:4.2} mi", total_distance / METERS_PER_MILE),
            format!("{} cal", total_calories)
        ));

        match sport.as_str() {
            "running" | "walking" => {
                tmp_vec.push(format!(
                    "{:10} ",
                    format!(
                        "{} / mi",
                        print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)?
                    )
                ));
                tmp_vec.push(format!(
                    "{:10} ",
                    format!(
                        "{} / km",
                        print_h_m_s(total_duration / (total_distance / 1000.), false)?
                    )
                ));
            }
            "biking" => {
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{:.2} mph",
                        (total_distance / METERS_PER_MILE) / (total_duration / 60. / 60.)
                    )
                ));
            }
            _ => (),
        };

        tmp_vec.push(format!(" {:10} \t", print_h_m_s(total_duration, true)?));
        if total_hr_dur > total_hr_dis {
            tmp_vec.push(format!(
                " {:7} {:2}",
                format!("{} bpm", (total_hr_dur / total_hr_dis) as i32),
                ""
            ));
        } else {
            tmp_vec.push(format!(" {:7} {:2}", "", ""));
        }

        tmp_vec.push(format!(
            "{:16}",
            format!("{} / {} days", number_of_days, total_days)
        ));

        result_vec.push(tmp_vec.join(" "));
    }
    Ok(result_vec)
}
