extern crate rayon;

use failure::Error;
use postgres::{Connection, TlsMode};

use crate::reports::garmin_report_options::GarminReportOptions;
use crate::utils::garmin_util::{
    days_in_month, days_in_year, print_h_m_s, METERS_PER_MILE, MONTH_NAMES, WEEKDAY_NAMES,
};
use crate::utils::sport_types::get_sport_type_string_map;

pub fn get_list_of_files_from_db(
    pg_url: &str,
    constraints: &Vec<String>,
) -> Result<Vec<String>, Error> {
    let constr = match constraints.len() {
        0 => "".to_string(),
        _ => format!("WHERE {}", constraints.join(" OR ")),
    };

    let query = format!("SELECT filename FROM garmin_summary {}", constr);

    let conn = Connection::connect(pg_url, TlsMode::None).unwrap();

    let file_list: Vec<String> = conn
        .query(&query, &[])?
        .iter()
        .map(|row| row.get(0))
        .collect();
    Ok(file_list)
}

pub fn create_report_query(
    pg_url: &str,
    options: &GarminReportOptions,
    constraints: &Vec<String>,
) -> Vec<String> {
    let conn = Connection::connect(pg_url, TlsMode::None).unwrap();

    let sport_type_string_map = get_sport_type_string_map();

    let sport_constr = match options.do_sport {
        Some(x) => match sport_type_string_map.get(&x) {
            Some(s) => format!("sport = '{}'", s),
            None => "".to_string(),
        },
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

    let result_vec = if options.do_year {
        year_summary_report(&conn, &constr)
    } else if options.do_month {
        month_summary_report(&conn, &constr)
    } else if options.do_week {
        week_summary_report(&conn, &constr)
    } else if options.do_day {
        day_summary_report(&conn, &constr)
    } else if options.do_file {
        file_summary_report(&conn, &constr)
    } else {
        vec!["".to_string()]
    };

    result_vec
}

fn file_summary_report(conn: &Connection, constr: &str) -> Vec<String> {
    let mut result_vec = Vec::new();
    let query = format!(
        "
        SELECT
            begin_datetime as datetime,
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            EXTRACT(isodow from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as isodow,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            sum(number_of_items) as number_of_items
        FROM garmin_summary
        {}
        GROUP BY sport, datetime, week, isodow
        ORDER BY sport, datetime, week, isodow
    ",
        constr
    );

    debug!("{}", query);

    for row in conn.query(&query, &[]).unwrap().iter() {
        let datetime: String = row.get(0);
        let week: f64 = row.get(1);
        let dow: f64 = row.get(2);
        let sport: String = row.get(3);
        let total_calories: i64 = row.get(4);
        let total_distance: f64 = row.get(5);
        let total_duration: f64 = row.get(6);
        let total_hr_dur: f64 = row.get(7);
        let total_hr_dis: f64 = row.get(8);
        let number_of_items: i64 = row.get(9);

        let weekdayname = WEEKDAY_NAMES[dow as usize - 1];

        debug!(
            "{} {} {} {} {} {} {} {} {} {}",
            datetime,
            week,
            dow,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
            number_of_items
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
                            print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)
                                .unwrap()
                        ),
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false).unwrap()
                        ),
                        print_h_m_s(total_duration, true).unwrap()
                    ));
                } else {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", datetime, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        format!(""),
                        format!(""),
                        print_h_m_s(total_duration, true).unwrap()
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
                    format!(""),
                    print_h_m_s(total_duration, true).unwrap()
                ));
            }
            _ => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", datetime, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    format!(""),
                    format!(""),
                    print_h_m_s(total_duration, true).unwrap()
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
    result_vec
}

fn day_summary_report(conn: &Connection, constr: &str) -> Vec<String> {
    let mut result_vec = Vec::new();
    let query = format!(
        "
        SELECT
            CAST(CAST(CAST(begin_datetime as timestamp with time zone) at time zone 'EST' as date) as text) as date,
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            EXTRACT(isodow from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as isodow,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            sum(number_of_items) as number_of_items
        FROM garmin_summary
        {}
        GROUP BY sport, date, week, isodow
        ORDER BY sport, date, week, isodow
    ",
        constr
    );

    debug!("{}", query);

    for row in conn.query(&query, &[]).unwrap().iter() {
        let date: String = row.get(0);
        let week: f64 = row.get(1);
        let dow: f64 = row.get(2);
        let sport: String = row.get(3);
        let total_calories: i64 = row.get(4);
        let total_distance: f64 = row.get(5);
        let total_duration: f64 = row.get(6);
        let total_hr_dur: f64 = row.get(7);
        let total_hr_dis: f64 = row.get(8);
        let number_of_items: i64 = row.get(9);

        let weekdayname = WEEKDAY_NAMES[dow as usize - 1];

        debug!(
            "{} {} {} {} {} {} {} {} {} {}",
            date,
            week,
            dow,
            sport,
            total_calories,
            total_distance,
            total_duration,
            total_hr_dur,
            total_hr_dis,
            number_of_items
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
                            print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)
                                .unwrap()
                        ),
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false).unwrap()
                        ),
                        print_h_m_s(total_duration, true).unwrap()
                    ));
                } else {
                    tmp_vec.push(format!(
                        "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                        format!("{:10} {:02} {:3}", date, week, weekdayname),
                        sport,
                        format!("{:.2} mi", total_distance / METERS_PER_MILE),
                        format!("{} cal", total_calories),
                        format!(""),
                        format!(""),
                        print_h_m_s(total_duration, true).unwrap()
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
                    format!(""),
                    print_h_m_s(total_duration, true).unwrap()
                ));
            }
            _ => {
                tmp_vec.push(format!(
                    "{:17} {:10} {:10} {:10} {:10} {:10} {:10}",
                    format!("{:10} {:02} {:3}", date, week, weekdayname),
                    sport,
                    format!("{:.2} mi", total_distance / METERS_PER_MILE),
                    format!("{} cal", total_calories),
                    format!(""),
                    format!(""),
                    print_h_m_s(total_duration, true).unwrap()
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
    result_vec
}

fn week_summary_report(conn: &Connection, constr: &str) -> Vec<String> {
    let mut result_vec = Vec::new();
    let query = format!("
        SELECT
            EXTRACT(isoyear from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as week,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            sum(number_of_items) as number_of_items,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM garmin_summary
        {}
        GROUP BY sport, year, week
        ORDER BY sport, year, week
    ", constr);

    debug!("{}", query);

    for row in conn.query(&query, &[]).unwrap().iter() {
        let year: f64 = row.get(0);
        let week: f64 = row.get(1);
        let sport: String = row.get(2);
        let total_calories: i64 = row.get(3);
        let total_distance: f64 = row.get(4);
        let total_duration: f64 = row.get(5);
        let total_hr_dur: f64 = row.get(6);
        let total_hr_dis: f64 = row.get(7);
        let number_of_items: i64 = row.get(8);
        let number_of_days: i64 = row.get(9);

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
            number_of_items
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
                            print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)
                                .unwrap()
                        )
                    ));
                    tmp_vec.push(format!(
                        " {:10} \t",
                        format!(
                            "{} / km",
                            print_h_m_s(total_duration / (total_distance / 1000.), false).unwrap()
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
        tmp_vec.push(format!(
            " {:10} \t",
            print_h_m_s(total_duration, true).unwrap()
        ));
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
    result_vec
}

fn month_summary_report(conn: &Connection, constr: &str) -> Vec<String> {
    let mut result_vec = Vec::new();
    let query = format!("
        SELECT
            EXTRACT(year from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            EXTRACT(month from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as month,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            sum(number_of_items) as number_of_items,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM garmin_summary
        {}
        GROUP BY sport, year, month
        ORDER BY sport, year, month
    ", constr);

    debug!("{}", query);

    for row in conn.query(&query, &[]).unwrap().iter() {
        let year: f64 = row.get(0);
        let month: f64 = row.get(1);
        let sport: String = row.get(2);
        let total_calories: i64 = row.get(3);
        let total_distance: f64 = row.get(4);
        let total_duration: f64 = row.get(5);
        let total_hr_dur: f64 = row.get(6);
        let total_hr_dis: f64 = row.get(7);
        let number_of_items: i64 = row.get(8);
        let number_of_days: i64 = row.get(9);

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
            number_of_items
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
                        print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)
                            .unwrap()
                    )
                ));
                tmp_vec.push(format!(
                    " {:10} \t",
                    format!(
                        "{} / km",
                        print_h_m_s(total_duration / (total_distance / 1000.), false).unwrap()
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
        tmp_vec.push(format!(
            " {:10} \t",
            print_h_m_s(total_duration, true).unwrap()
        ));

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
    result_vec
}

fn year_summary_report(conn: &Connection, constr: &str) -> Vec<String> {
    let mut result_vec = Vec::new();

    let query = format!(
        "
        SELECT
            EXTRACT(year from cast(begin_datetime as timestamp with time zone) at time zone 'EST') as year,
            sport,
            sum(total_calories) as total_calories,
            sum(total_distance) as total_distance,
            sum(total_duration) as total_duration,
            sum(total_hr_dur) as total_hr_dur,
            sum(total_hr_dis) as total_hr_dis,
            sum(number_of_items) as number_of_items,
            count(distinct cast(cast(begin_datetime as timestamp with time zone) at time zone 'EST' as date)) as number_of_days
        FROM garmin_summary
        {}
        GROUP BY sport, year
        ORDER BY sport, year
    ",
        constr
    );
    debug!("{}", query);

    for row in conn.query(&query, &[]).unwrap().iter() {
        let year: f64 = row.get(0);
        let sport: String = row.get(1);
        let total_calories: i64 = row.get(2);
        let total_distance: f64 = row.get(3);
        let total_duration: f64 = row.get(4);
        let total_hr_dur: f64 = row.get(5);
        let total_hr_dis: f64 = row.get(6);
        let number_of_items: i64 = row.get(7);
        let number_of_days: i64 = row.get(8);

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
            number_of_items
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
                        print_h_m_s(total_duration / (total_distance / METERS_PER_MILE), false)
                            .unwrap()
                    )
                ));
                tmp_vec.push(format!(
                    "{:10} ",
                    format!(
                        "{} / km",
                        print_h_m_s(total_duration / (total_distance / 1000.), false).unwrap()
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

        tmp_vec.push(format!(
            " {:10} \t",
            print_h_m_s(total_duration, true).unwrap()
        ));
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
    result_vec
}
