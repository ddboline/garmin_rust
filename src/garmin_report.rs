extern crate rayon;

use failure::Error;
use postgres::{Connection, TlsMode};
use std::env;
use std::fs::create_dir_all;
use std::fs::File;
use std::io::prelude::*;

use subprocess::Exec;

use rayon::prelude::*;

use crate::garmin_file::GarminFile;
use crate::garmin_lap::GarminLap;
use crate::garmin_templates::{GARMIN_TEMPLATE, MAP_TEMPLATE};
use crate::garmin_util::{
    days_in_month, days_in_year, get_sport_type_map, get_sport_type_string_map, plot_graph,
    print_h_m_s, titlecase, PlotOpts, SportTypes, MARATHON_DISTANCE_MI, METERS_PER_MILE,
    MONTH_NAMES, WEEKDAY_NAMES,
};

#[derive(Debug, Clone)]
pub struct GarminReportOptions {
    pub do_year: bool,
    pub do_month: bool,
    pub do_week: bool,
    pub do_day: bool,
    pub do_file: bool,
    pub do_sport: Option<SportTypes>,
}

impl GarminReportOptions {
    pub fn new() -> GarminReportOptions {
        GarminReportOptions {
            do_year: false,
            do_month: false,
            do_week: false,
            do_day: false,
            do_file: false,
            do_sport: None,
        }
    }
}

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

    let file_list: Vec<String> = conn.query(&query, &[])?
        .iter()
        .map(|row| row.get(0))
        .collect();
    Ok(file_list)
}

pub fn generate_txt_report(gfile: &GarminFile) -> Vec<String> {
    let mut return_vec = vec![format!("Start time {}", gfile.filename)];

    let sport_type_map = get_sport_type_map();
    let sport_type = match &gfile.sport {
        Some(sport) => match sport_type_map.get(sport) {
            Some(s) => s.clone(),
            None => SportTypes::Other,
        },
        None => SportTypes::Other,
    };

    for lap in &gfile.laps {
        return_vec.push(print_lap_string(&lap, &sport_type))
    }

    let mut min_mile = 0.0;
    let mut mi_per_hr = 0.0;
    if gfile.total_distance > 0.0 {
        min_mile = (gfile.total_duration / 60.) / (gfile.total_distance / METERS_PER_MILE);
    };
    if gfile.total_duration > 0.0 {
        mi_per_hr = (gfile.total_distance / METERS_PER_MILE) / (gfile.total_duration / 60. / 60.);
    };

    let mut tmp_str = Vec::new();
    match sport_type {
        SportTypes::Running => {
            tmp_str.push(format!(
                "total {:.2} mi {} calories {} time {} min/mi {} min/km",
                gfile.total_distance / METERS_PER_MILE,
                gfile.total_calories,
                print_h_m_s(gfile.total_duration, true).unwrap(),
                print_h_m_s(min_mile * 60.0, false).unwrap(),
                print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false).unwrap()
            ));
        }
        _ => {
            tmp_str.push(format!(
                "total {:.2} mi {} calories {} time {} mph",
                gfile.total_distance / METERS_PER_MILE,
                gfile.total_calories,
                print_h_m_s(gfile.total_duration, true).unwrap(),
                mi_per_hr
            ));
        }
    };
    if gfile.total_hr_dur > gfile.total_hr_dis {
        tmp_str.push(format!(
            "{:.2} bpm",
            (gfile.total_hr_dur / gfile.total_hr_dis) as i32
        ));
    };
    return_vec.push(tmp_str.join(" "));
    return_vec.push("".to_string());
    return_vec.push(print_splits(&gfile, METERS_PER_MILE, "mi"));
    return_vec.push("".to_string());
    return_vec.push(print_splits(&gfile, 5000.0, "km"));

    let avg_hr: f64 = gfile
        .points
        .iter()
        .map(|point| match point.heart_rate {
            Some(hr) => {
                if hr > 0.0 {
                    hr * point.duration_from_last
                } else {
                    0.0
                }
            }
            None => 0.0,
        })
        .sum();
    let sum_time: f64 = gfile
        .points
        .iter()
        .map(|point| match point.heart_rate {
            Some(hr) => {
                if hr > 0.0 {
                    point.duration_from_last
                } else {
                    0.0
                }
            }
            None => 0.0,
        })
        .sum();
    let hr_vals: Vec<_> = gfile
        .points
        .iter()
        .map(|point| match point.heart_rate {
            Some(hr) => {
                if hr > 0.0 {
                    hr
                } else {
                    0.0
                }
            }
            None => 0.0,
        })
        .collect();

    let avg_hr = if sum_time > 0.0 {
        avg_hr / sum_time
    } else {
        avg_hr
    };

    if (sum_time > 0.0) & (hr_vals.len() > 0) {
        return_vec.push("".to_string());
        return_vec.push(format!(
            "Heart Rate {:2.2} avg {:2.2} max",
            avg_hr,
            hr_vals.iter().map(|x| *x as i32).max().unwrap()
        ));
    }

    let mut vertical_climb = 0.0;
    let mut cur_alt = 0.0;
    let mut last_alt = 0.0;

    let alt_vals: Vec<_> = gfile
        .points
        .iter()
        .filter_map(|point| match point.altitude {
            Some(alt) => {
                if (alt > 0.0) & (alt < 10000.0) {
                    cur_alt = alt;
                    vertical_climb += cur_alt - last_alt;
                    last_alt = cur_alt;
                    Some(alt)
                } else {
                    None
                }
            }
            None => None,
        })
        .collect();

    if alt_vals.len() > 0 {
        return_vec.push(format!(
            "max altitude diff: {:.2} m",
            alt_vals.iter().map(|x| *x as i32).max().unwrap()
                - alt_vals.iter().map(|x| *x as i32).min().unwrap()
        ));
        return_vec.push(format!("vertical climb: {:.2} m", vertical_climb));
    }

    return_vec
}

pub fn print_lap_string(glap: &GarminLap, sport: &SportTypes) -> String {
    let sport_map = get_sport_type_string_map();
    let sport_str = match sport_map.get(sport) {
        Some(s) => s.clone(),
        None => "other".to_string(),
    };
    let mut outstr = vec![format!(
        "{} lap {} {:.2} mi {} {} calories {:.2} min",
        sport_str,
        glap.lap_number,
        glap.lap_distance / METERS_PER_MILE,
        print_h_m_s(glap.lap_duration, true).unwrap(),
        glap.lap_calories,
        glap.lap_duration / 60.
    )];

    if (*sport == SportTypes::Running) & (glap.lap_distance > 0.0) {
        outstr.push(
            print_h_m_s(
                glap.lap_duration / (glap.lap_distance / METERS_PER_MILE),
                false,
            ).unwrap(),
        );
        outstr.push("/ mi ".to_string());
        outstr.push(print_h_m_s(glap.lap_duration / (glap.lap_distance / 1000.), false).unwrap());
        outstr.push("/ km".to_string());
    };
    if let Some(x) = glap.lap_avg_hr {
        if x > 0.0 {
            outstr.push(format!("{} bpm", x));
        }
    }

    outstr.join(" ")
}

pub fn print_splits(gfile: &GarminFile, split_distance_in_meters: f64, label: &str) -> String {
    if gfile.points.len() == 0 {
        return "".to_string();
    };

    let split_vector = get_splits(gfile, split_distance_in_meters, label, true);
    let retval: Vec<_> = split_vector
        .iter()
        .map(|val| {
            let dis = *val.get(0).unwrap() as i32;
            let tim = *val.get(1).unwrap();
            let hrt = *val.get(2).unwrap_or(&0.0);

            format!(
                "{} {} \t {} \t {} / mi \t {} / km \t {} \t {:.2} bpm avg",
                dis,
                label,
                print_h_m_s(tim, true).unwrap(),
                print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false).unwrap(),
                print_h_m_s(tim / (split_distance_in_meters / 1000.), false).unwrap(),
                print_h_m_s(
                    tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                    true
                ).unwrap(),
                hrt
            )
        })
        .collect();
    retval.join("\n")
}

pub fn get_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
    do_heart_rate: bool,
) -> Vec<Vec<f64>> {
    if gfile.points.len() < 3 {
        return Vec::new();
    };
    let mut last_point_me = Some(0.0);
    let mut last_point_time = 0.0;
    let mut prev_split_time = 0.0;
    let mut avg_hrt_rate = 0.0;

    let mut split_vector = Vec::new();

    for point in &gfile.points {
        let cur_point_me = point.distance;
        let cur_point_time = point.duration_from_begin;
        if (cur_point_me == None) | (last_point_me == None) {
            continue;
        }
        if (cur_point_me.unwrap() - last_point_me.unwrap()) <= 0.0 {
            continue;
        }
        match point.heart_rate {
            Some(hr) => avg_hrt_rate += hr * (cur_point_time - last_point_time),
            _ => (),
        }
        let nmiles = (cur_point_me.unwrap() / split_distance_in_meters) as i32
            - (last_point_me.unwrap() / split_distance_in_meters) as i32;
        if nmiles > 0 {
            let cur_split_me = (cur_point_me.unwrap() / split_distance_in_meters) as i32;
            let cur_split_me = cur_split_me as f64 * split_distance_in_meters;

            debug!(
                "get splits 0 {} {} {} {} {} {} ",
                &last_point_time,
                &cur_point_time,
                &cur_point_me.unwrap(),
                &last_point_me.unwrap(),
                &cur_split_me,
                &last_point_me.unwrap()
            );

            let cur_split_time = last_point_time
                + (cur_point_time - last_point_time)
                    / (cur_point_me.unwrap() - last_point_me.unwrap())
                    * (cur_split_me - last_point_me.unwrap());
            let time_val = cur_split_time - prev_split_time;
            let split_dist = if label == "km" {
                cur_point_me.unwrap() / 1000.
            } else {
                cur_point_me.unwrap() / split_distance_in_meters
            };
            let tmp_vector = if do_heart_rate {
                vec![
                    split_dist,
                    time_val,
                    avg_hrt_rate / (cur_split_time - prev_split_time),
                ]
            } else {
                vec![split_dist, time_val]
            };

            debug!("get splits 1 {:?}", &tmp_vector);

            split_vector.push(tmp_vector);

            prev_split_time = cur_split_time.clone();
            avg_hrt_rate = 0.0;
        };
        last_point_me = cur_point_me;
        last_point_time = cur_point_time;
    }
    split_vector
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

pub fn file_report_html(
    gfile: &GarminFile,
    maps_api_key: &str,
    cache_dir: &str,
) -> Result<String, Error> {
    let sport = match &gfile.sport {
        Some(s) => s.clone(),
        None => "none".to_string(),
    };

    let mut avg_hr = 0.0;
    let mut sum_time = 0.0;
    let mut max_hr = 0.0;

    let mut hr_vals = Vec::new();
    let mut hr_values = Vec::new();
    let mut alt_vals = Vec::new();
    let mut alt_values = Vec::new();
    let mut mph_speed_values = Vec::new();
    let mut avg_speed_values = Vec::new();
    let mut avg_mph_speed_values = Vec::new();
    let mut lat_vals = Vec::new();
    let mut lon_vals = Vec::new();

    let home_dir = env::var("HOME").unwrap();

    let speed_values = get_splits(&gfile, 400., "lap", true);
    let heart_rate_speed: Vec<_> = speed_values
        .iter()
        .map(|v| {
            let t = v.get(1).unwrap();
            let h = v.get(2).unwrap();
            (*h, 4.0 * t / 60.)
        })
        .collect();
    let speed_values: Vec<_> = speed_values
        .into_iter()
        .map(|v| {
            let d = v.get(0).unwrap();
            let t = v.get(1).unwrap();
            (d / 4., 4. * t / 60.)
        })
        .collect();
    let mile_split_vals = get_splits(&gfile, METERS_PER_MILE, "mi", false);
    let mile_split_vals: Vec<_> = mile_split_vals
        .into_iter()
        .map(|v| {
            let d = v.get(0).unwrap();
            let t = v.get(1).unwrap();
            (*d, t / 60.)
        })
        .collect();

    for point in &gfile.points {
        if point.distance == None {
            continue;
        }
        let xval = point.distance.unwrap_or(0.0) / METERS_PER_MILE;
        if xval > 0.0 {
            if let Some(hr) = point.heart_rate {
                if hr > 0.0 {
                    avg_hr += hr * point.duration_from_last;
                    sum_time += point.duration_from_last;
                    hr_vals.push(hr);
                    hr_values.push((xval, hr));
                }
            }
        };
        if let Some(alt) = point.altitude {
            if (alt > 0.0) & (alt < 10000.0) {
                alt_vals.push(alt);
                alt_values.push((xval, alt));
            }
        };
        if (point.speed_mph > 0.0) & (point.speed_mph < 20.0) {
            mph_speed_values.push((xval, point.speed_mph));
        };
        if (point.avg_speed_value_permi > 0.0) & (point.avg_speed_value_permi < 20.0) {
            avg_speed_values.push((xval, point.avg_speed_value_permi));
        };
        if point.avg_speed_value_mph > 0.0 {
            avg_mph_speed_values.push((xval, point.avg_speed_value_mph));
        };
        if let Some(lat) = point.latitude {
            if let Some(lon) = point.longitude {
                lat_vals.push(lat);
                lon_vals.push(lon);
            }
        };
    }
    if sum_time > 0.0 {
        avg_hr /= sum_time;
        max_hr = hr_vals.iter().map(|h| *h as i64).max().unwrap() as f64;
    };

    let mut plot_opts = Vec::new();

    if mile_split_vals.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("mile_splits")
                .with_title("Pace per Mile every mi")
                .with_data(&mile_split_vals)
                .with_marker("o")
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir),
        );
    };

    if hr_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heart_rate")
                .with_title(format!("Heart Rate {:2.2} avg {:2.2} max", avg_hr, max_hr).as_str())
                .with_data(&hr_values)
                .with_labels("mi", "bpm")
                .with_cache_dir(&cache_dir),
        );
    };

    if alt_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("altitude")
                .with_title("Altitude")
                .with_data(&alt_values)
                .with_labels("mi", "height [m]")
                .with_cache_dir(&cache_dir),
        );
    };

    if speed_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_minpermi")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&speed_values)
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir),
        );

        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_mph")
                .with_title("Speed mph")
                .with_data(&mph_speed_values)
                .with_labels("mi", "mph")
                .with_cache_dir(&cache_dir),
        );
    };

    if heart_rate_speed.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heartrate_vs_speed")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&heart_rate_speed)
                .with_scatter()
                .with_labels("hrt", "min/mi")
                .with_cache_dir(&cache_dir),
        );
    };

    if avg_speed_values.len() > 0 {
        let (_, avg_speed_value) = avg_speed_values.last().unwrap();
        let avg_speed_value_min = *avg_speed_value as i32;
        let avg_speed_value_sec = ((*avg_speed_value - avg_speed_value_min as f64) * 60.0) as i32;

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_minpermi")
                .with_title(
                    format!(
                        "Avg Speed {}:{:02} min/mi",
                        avg_speed_value_min, avg_speed_value_sec
                    ).as_str(),
                )
                .with_data(&heart_rate_speed)
                .with_scatter()
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir),
        );
    };

    if avg_mph_speed_values.len() > 0 {
        let (_, avg_mph_speed_value) = avg_mph_speed_values.last().unwrap();

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_mph")
                .with_title(format!("Avg Speed {:.2} mph", avg_mph_speed_value).as_str())
                .with_data(&avg_mph_speed_values)
                .with_scatter()
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir),
        );
    };

    Exec::shell(format!("rm -rf {}/html", cache_dir)).join()?;

    let graphs: Vec<_> = plot_opts
        .par_iter()
        .filter_map(|options| match plot_graph(&options) {
            Ok(x) => Some(x),
            Err(err) => {
                println!("{}", err);
                None
            }
        })
        .collect();

    let mut htmlfile = File::create(&format!("{}/html/index.html", cache_dir))?;

    if (lat_vals.len() > 0) & (lon_vals.len() > 0) & (lat_vals.len() == lon_vals.len()) {
        let minlat = lat_vals.iter().map(|v| (v * 1000.0) as i32).min().unwrap() as f64 / 1000.0;
        let maxlat = lat_vals.iter().map(|v| (v * 1000.0) as i32).max().unwrap() as f64 / 1000.0;
        let minlon = lon_vals.iter().map(|v| (v * 1000.0) as i32).min().unwrap() as f64 / 1000.0;
        let maxlon = lon_vals.iter().map(|v| (v * 1000.0) as i32).max().unwrap() as f64 / 1000.0;
        let central_lat = (maxlat + minlat) / 2.0;
        let central_lon = (maxlon + minlon) / 2.0;
        let latlon_min = if (maxlat - minlat) > (maxlon - minlon) {
            maxlat - minlat
        } else {
            maxlon - minlon
        };
        let latlon_thresholds = vec![
            (15, 0.015),
            (14, 0.038),
            (13, 0.07),
            (12, 0.12),
            (11, 0.20),
            (10, 0.4),
        ];

        for line in MAP_TEMPLATE.split("\n") {
            if line.contains("SPORTTITLEDATE") {
                let newtitle = format!(
                    "Garmin Event {} on {}",
                    titlecase(&sport),
                    gfile.begin_datetime
                );
                write!(htmlfile, "{}", line.replace("SPORTTITLEDATE", &newtitle));
            } else if line.contains("ZOOMVALUE") {
                for (zoom, thresh) in &latlon_thresholds {
                    if (latlon_min < *thresh) | (*zoom == 10) {
                        write!(
                            htmlfile,
                            "{}",
                            line.replace("ZOOMVALUE", &format!("{}", zoom))
                        );
                        break;
                    }
                }
            } else if line.contains("INSERTTABLESHERE") {
                write!(htmlfile, "{}\n", get_file_html(&gfile));
                write!(
                    htmlfile,
                    "<br><br>{}\n",
                    get_html_splits(&gfile, METERS_PER_MILE, "mi")
                );
                write!(
                    htmlfile,
                    "<br><br>{}\n",
                    get_html_splits(&gfile, 5000.0, "km")
                );
            } else if line.contains("INSERTMAPSEGMENTSHERE") {
                for (latv, lonv) in lat_vals.iter().zip(lon_vals.iter()) {
                    write!(htmlfile, "new google.maps.LatLng({},{}),\n", latv, lonv);
                }
            } else if line.contains("MINLAT") | line.contains("MAXLAT") | line.contains("MINLON")
                | line.contains("MAXLON")
            {
                write!(
                    htmlfile,
                    "{}",
                    line.replace("MINLAT", &format!("{}", minlat))
                        .replace("MAXLAT", &format!("{}", maxlat))
                        .replace("MINLON", &format!("{}", minlon))
                        .replace("MAXLON", &format!("{}", maxlon))
                );
            } else if line.contains("CENTRALLAT") | line.contains("CENTRALLON") {
                write!(
                    htmlfile,
                    "{}",
                    line.replace("CENTRALLAT", &format!("{}", central_lat))
                        .replace("CENTRALLON", &format!("{}", central_lon))
                );
            } else if line.contains("INSERTOTHERIMAGESHERE") {
                for gf in &graphs {
                    write!(htmlfile, "{}{}{}", r#"<p><img src=""#, gf, r#""></p>"#);
                }
            } else if line.contains("MAPSAPIKEY") {
                write!(htmlfile, "{}", line.replace("MAPSAPIKEY", maps_api_key));
            } else {
                write!(htmlfile, "{}", line);
            };
        }
    } else {
        for line in GARMIN_TEMPLATE.split("\n") {
            if line.contains("INSERTTEXTHERE") {
                write!(htmlfile, "{}\n", get_file_html(&gfile));
                write!(
                    htmlfile,
                    "<br><br>{}\n",
                    get_html_splits(&gfile, METERS_PER_MILE, "mi")
                );
                write!(
                    htmlfile,
                    "<br><br>{}\n",
                    get_html_splits(&gfile, 5000.0, "km")
                );
            } else if line.contains("SPORTTITLEDATE") {
                let newtitle = format!(
                    "Garmin Event {} on {}",
                    titlecase(&sport),
                    gfile.begin_datetime
                );
                write!(htmlfile, "{}", line.replace("SPORTTITLEDATE", &newtitle));
            } else {
                write!(
                    htmlfile,
                    "{}",
                    line.replace("<pre>", "<div>").replace("</pre>", "</div>")
                );
            }
        }
    };

    Exec::shell(format!("rm -rf {}/public_html/garmin/html", home_dir)).join()?;
    Exec::shell(format!(
        "mv {}/html {}/public_html/garmin",
        cache_dir, home_dir
    )).join()?;

    Ok(format!("{}/html", cache_dir))
}

fn get_file_html(gfile: &GarminFile) -> String {
    let mut retval = Vec::new();

    let sport = match &gfile.sport {
        Some(s) => s.clone(),
        None => "none".to_string(),
    };

    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(
        r#"<thead><tr style="text-align: center;"><th>Start Time</th>
                   <th>Sport</th></tr></thead>"#.to_string(),
    );
    retval.push(format!(
        "<tbody><tr style={0}text-align: center;{0}><td>{1}</td><td>{2}</td></tr></tbody>",
        '"', gfile.begin_datetime, sport
    ));
    retval.push(r#"</table><br>"#.to_string());

    let labels = vec![
        "Sport",
        "Lap",
        "Distance",
        "Duration",
        "Calories",
        "Time",
        "Pace / mi",
        "Pace / km",
        "Heart Rate",
    ];
    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());
    for label in labels {
        retval.push(format!("<th>{}</th>", label));
    }
    retval.push("</tr></thead>".to_string());
    retval.push("<tbody>".to_string());
    for lap in &gfile.laps {
        retval.push(r#"<tr style="text-align: center;">"#.to_string());
        for lap_html in get_lap_html(&lap, &sport) {
            retval.push(lap_html);
        }
        retval.push("</tr>".to_string());
    }

    let min_mile = if gfile.total_distance > 0.0 {
        (gfile.total_duration / 60.) / (gfile.total_distance / METERS_PER_MILE)
    } else {
        0.0
    };

    let mi_per_hr = if gfile.total_duration > 0.0 {
        (gfile.total_distance / METERS_PER_MILE) / (gfile.total_duration / 3600.)
    } else {
        0.0
    };

    let (mut labels, mut values) = match sport.as_str() {
        "running" => (
            vec![
                "".to_string(),
                "Distance".to_string(),
                "Calories".to_string(),
                "Time".to_string(),
                "Pace / mi".to_string(),
                "Pace / km".to_string(),
            ],
            vec![
                "total".to_string(),
                format!("{:.2} mi", gfile.total_distance / METERS_PER_MILE),
                format!("{}", gfile.total_calories),
                print_h_m_s(gfile.total_duration, true).unwrap(),
                print_h_m_s(min_mile * 60.0, false).unwrap(),
                print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false).unwrap(),
            ],
        ),
        _ => (
            vec![
                "total".to_string(),
                "Distance".to_string(),
                "Calories".to_string(),
                "Time".to_string(),
                "Pace mph".to_string(),
            ],
            vec![
                "".to_string(),
                format!("{:.2} mi", gfile.total_distance / METERS_PER_MILE),
                format!("{}", gfile.total_calories),
                print_h_m_s(gfile.total_duration, true).unwrap(),
                format!("{}", mi_per_hr),
            ],
        ),
    };

    if (gfile.total_hr_dur > 0.0) & (gfile.total_hr_dis > 0.0)
        & (gfile.total_hr_dur > gfile.total_hr_dis)
    {
        labels.push("Heart Rate".to_string());
        values.push(format!(
            "{} bpm",
            (gfile.total_hr_dur / gfile.total_hr_dis) as i32
        ));
    };

    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());

    for label in labels {
        retval.push(format!("<th>{}</th>", label));
    }

    retval.push("</tr></thead>".to_string());
    retval.push(r#"<tbody><tr style="text-align: center;">"#.to_string());

    for value in values {
        retval.push(format!("<td>{}</td>", value));
    }

    retval.push("</tr></tbody></table>".to_string());

    retval.join("\n")
}

fn get_lap_html(glap: &GarminLap, sport: &str) -> Vec<String> {
    let mut values = vec![
        sport.to_string(),
        format!("{}", glap.lap_number),
        format!("{:.2} mi", glap.lap_distance / METERS_PER_MILE),
        print_h_m_s(glap.lap_duration, true).unwrap(),
        format!("{}", glap.lap_calories),
        format!("{:.2} min", glap.lap_duration / 60.),
    ];
    if glap.lap_distance > 0.0 {
        values.push(format!(
            "{} / mi",
            print_h_m_s(
                glap.lap_duration / (glap.lap_distance / METERS_PER_MILE),
                false
            ).unwrap()
        ));
        values.push(format!(
            "{} / km",
            print_h_m_s(glap.lap_duration / (glap.lap_distance / 1000.), false).unwrap()
        ));
    }
    if let Some(lap_avg_hr) = glap.lap_avg_hr {
        values.push(format!("{} bpm", lap_avg_hr));
    }
    values.iter().map(|v| format!("<td>{}</td>", v)).collect()
}

fn get_html_splits(gfile: &GarminFile, split_distance_in_meters: f64, label: &str) -> String {
    if gfile.points.len() == 0 {
        "".to_string()
    } else {
        let labels = vec![
            "Split",
            "Time",
            "Pace / mi",
            "Pace / km",
            "Marathon Time",
            "Heart Rate",
        ];

        let split_vector = get_splits(gfile, split_distance_in_meters, label, true);

        let values: Vec<_> = split_vector
            .iter()
            .map(|val| {
                let dis = *val.get(0).unwrap() as i32;
                let tim = val.get(1).unwrap();
                let hrt = *val.get(2).unwrap_or(&0.0) as i32;
                vec![
                    format!("{} {}", dis, label),
                    print_h_m_s(*tim, true).unwrap(),
                    print_h_m_s(*tim / (split_distance_in_meters / METERS_PER_MILE), false)
                        .unwrap(),
                    print_h_m_s(*tim / (split_distance_in_meters / 1000.), false).unwrap(),
                    print_h_m_s(
                        *tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                        true,
                    ).unwrap(),
                    format!("{} bpm", hrt),
                ]
            })
            .collect();

        let mut retval = Vec::new();
        retval.push(r#"<table border="1" class="dataframe">"#.to_string());
        retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());
        for label in labels {
            retval.push(format!("<th>{}</th>", label));
        }
        retval.push("</tr></thead>".to_string());
        retval.push("<tbody>".to_string());
        for line in values {
            retval.push(r#"<tr style="text-align: center;">"#.to_string());
            for val in line {
                retval.push(format!("<td>{}</td>", val));
            }
            retval.push("</tr>".to_string());
        }
        retval.push("</tbody></table>".to_string());
        retval.join("\n")
    }
}

pub fn summary_report_html(
    retval: &Vec<String>,
    cmd_args: &mut Vec<String>,
    cache_dir: &str,
) -> Result<(), Error> {
    let home_dir = env::var("HOME").unwrap();

    let htmlostr: Vec<_> = retval
        .iter()
        .map(|ent| match cmd_args.pop() {
            Some(cmd) => format!(
                "{}{}{}{}{}{}",
                r#"<button type="submit" onclick="send_command('"#,
                cmd,
                r#");">"#,
                cmd,
                "</button> ",
                ent.trim()
            ),
            None => ent.to_string(),
        })
        .collect();

    let htmlostr = htmlostr.join("\n").replace("\n\n", "<br>\n");

    create_dir_all(&format!("{}/html", cache_dir))?;

    let mut htmlfile = File::create(&format!("{}/html/index.html", cache_dir))?;

    for line in GARMIN_TEMPLATE.split("\n") {
        if line.contains("INSERTTEXTHERE") {
            write!(htmlfile, "{}", htmlostr);
        } else if line.contains("SPORTTITLEDATE") {
            let newtitle = "Garmin Summary";
            write!(htmlfile, "{}", line.replace("SPORTTITLEDATE", newtitle));
        } else {
            write!(htmlfile, "{}", line);
        }
    }

    create_dir_all(&format!("{}/html/garmin", home_dir))?;
    Exec::shell(format!("rm -rf {}/public_html/garmin/html", home_dir)).join()?;
    Exec::shell(format!(
        "mv {}/html {}/public_html/garmin/",
        cache_dir, home_dir
    )).join()?;
    Ok(())
}
