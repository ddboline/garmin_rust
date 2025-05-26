use itertools::Itertools;
use log::debug;
use stack_string::{format_sstr, StackString};

use garmin_lib::errors::GarminError as Error;
use garmin_models::{garmin_file::GarminFile, garmin_lap::GarminLap};
use garmin_utils::{
    garmin_util::{print_h_m_s, MARATHON_DISTANCE_MI, METERS_PER_MILE},
    sport_types::SportTypes,
};

/// # Errors
/// Returns error if we try to parse a negative duration or time
pub fn generate_txt_report(gfile: &GarminFile) -> Result<Vec<StackString>, Error> {
    let mut return_vec = vec![format_sstr!("Start time {}", gfile.filename)];

    let sport_type = gfile.sport;

    for lap in &gfile.laps {
        return_vec.push(print_lap_string(lap, sport_type)?);
    }

    let mut min_mile = 0.0;
    let mut mi_per_hr = 0.0;
    if gfile.total_distance > 0.0 {
        min_mile = (gfile.total_duration / 60.) / (gfile.total_distance / METERS_PER_MILE);
    }
    if gfile.total_duration > 0.0 {
        mi_per_hr = (gfile.total_distance / METERS_PER_MILE) / (gfile.total_duration / 60. / 60.);
    }

    let mut tmp_str = Vec::new();
    if sport_type == SportTypes::Running {
        tmp_str.push(format_sstr!(
            "total {:.2} mi {} calories {} time {} min/mi {} min/km",
            gfile.total_distance / METERS_PER_MILE,
            gfile.total_calories,
            print_h_m_s(gfile.total_duration, true)?,
            print_h_m_s(min_mile * 60.0, false)?,
            print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false)?
        ));
    } else {
        tmp_str.push(format_sstr!(
            "total {:.2} mi {} calories {} time {} mph",
            gfile.total_distance / METERS_PER_MILE,
            gfile.total_calories,
            print_h_m_s(gfile.total_duration, true)?,
            format_sstr!("{mi_per_hr:.2}"),
        ));
    }

    if gfile.total_hr_dur > gfile.total_hr_dis {
        tmp_str.push(format_sstr!(
            "{:.2} bpm",
            (gfile.total_hr_dur / gfile.total_hr_dis) as i32
        ));
    }
    return_vec.push(tmp_str.join(" ").into());
    return_vec.push("".into());
    return_vec.push(print_splits(gfile, METERS_PER_MILE, "mi"));
    return_vec.push("".into());
    return_vec.push(print_splits(gfile, 5000.0, "km"));

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
    let mut hr_vals: Vec<_> = gfile
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
    hr_vals.shrink_to_fit();

    let avg_hr = if sum_time > 0.0 {
        avg_hr / sum_time
    } else {
        avg_hr
    };

    if (sum_time > 0.0) & !hr_vals.is_empty() {
        return_vec.push("".into());
        return_vec.push(format_sstr!(
            "Heart Rate {:2.2} avg {:2.2} max",
            avg_hr,
            hr_vals.iter().map(|x| *x as i32).max().unwrap_or(0)
        ));
    }

    let mut vertical_climb = 0.0;
    let mut cur_alt = 0.0;
    let mut last_alt = 0.0;

    let mut alt_vals: Vec<_> = gfile
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
    alt_vals.shrink_to_fit();

    if !alt_vals.is_empty() {
        return_vec.push(format_sstr!(
            "max altitude diff: {:.2} m",
            alt_vals.iter().map(|x| *x as i32).max().unwrap_or(0)
                - alt_vals.iter().map(|x| *x as i32).min().unwrap_or(0)
        ));
        return_vec.push(format_sstr!("vertical climb: {vertical_climb:.2} m"));
    }

    Ok(return_vec)
}

/// # Errors
/// Returns error if we try to parse a negative duration or time
fn print_lap_string(glap: &GarminLap, sport: SportTypes) -> Result<StackString, Error> {
    let sport_str: StackString = sport.into();

    let mut outstr = vec![format_sstr!(
        "{} lap {} {:.2} mi {} {} calories {:.2} min",
        sport_str,
        glap.lap_number,
        glap.lap_distance / METERS_PER_MILE,
        print_h_m_s(glap.lap_duration, true)?,
        glap.lap_calories,
        glap.lap_duration / 60.
    )];

    if (sport == SportTypes::Running) & (glap.lap_distance > 0.0) {
        outstr.push(print_h_m_s(
            glap.lap_duration / (glap.lap_distance / METERS_PER_MILE),
            false,
        )?);
        outstr.push("/ mi ".into());
        outstr.push(print_h_m_s(
            glap.lap_duration / (glap.lap_distance / 1000.),
            false,
        )?);
        outstr.push("/ km".into());
    }
    if let Some(x) = glap.lap_avg_hr {
        if x > 0.0 {
            outstr.push(format_sstr!("{x} bpm"));
        }
    }

    Ok(outstr.join(" ").into())
}

/// # Errors
/// Returns error if we try to parse a negative duration or time
fn print_splits(gfile: &GarminFile, split_distance_in_meters: f64, label: &str) -> StackString {
    if gfile.points.is_empty() {
        return "".into();
    }

    get_splits(gfile, split_distance_in_meters, label, true)
        .into_iter()
        .map(|val| {
            let dis = val.split_distance as i32;
            let tim = val.time_value;
            let hrt = val.avg_heart_rate.unwrap_or(0.0);

            format_sstr!(
                "{} {} \t {} \t {} / mi \t {} / km \t {} \t {:.2} bpm avg",
                dis,
                label,
                print_h_m_s(tim, true).unwrap_or_else(|_| "".into()),
                print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false)
                    .unwrap_or_else(|_| "".into()),
                print_h_m_s(tim / (split_distance_in_meters / 1000.), false)
                    .unwrap_or_else(|_| "".into()),
                print_h_m_s(
                    tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                    true
                )
                .unwrap_or_else(|_| "".into()),
                hrt
            )
        })
        .join("\n")
        .into()
}

#[derive(Debug)]
pub struct SplitValue {
    pub split_distance: f64,
    pub time_value: f64,
    pub avg_heart_rate: Option<f64>,
}

#[must_use]
pub fn get_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
    do_heart_rate: bool,
) -> Vec<SplitValue> {
    if gfile.points.len() < 3 {
        return Vec::new();
    }
    let mut last_point_me = 0.0;
    let mut last_point_time = 0.0;
    let mut prev_split_time = 0.0;
    let mut avg_hrt_rate = 0.0;

    let mut split_vector = Vec::new();

    for point in &gfile.points {
        let Some(cur_point_me) = point.distance else {
            continue;
        };
        let cur_point_time = point.duration_from_begin;
        if (cur_point_me - last_point_me) <= 0.0 {
            continue;
        }

        if let Some(hr) = point.heart_rate {
            avg_hrt_rate += hr * (cur_point_time - last_point_time);
        }

        let nmiles = (cur_point_me / split_distance_in_meters) as i32
            - (last_point_me / split_distance_in_meters) as i32;
        if nmiles > 0 {
            let cur_split_me = (cur_point_me / split_distance_in_meters) as i32;
            let cur_split_me = f64::from(cur_split_me) * split_distance_in_meters;

            debug!(
                "get splits 0 {} {} {} {} {} {} ",
                &last_point_time,
                &cur_point_time,
                &cur_point_me,
                &last_point_me,
                &cur_split_me,
                &last_point_me
            );

            let cur_split_time = last_point_time
                + (cur_point_time - last_point_time) / (cur_point_me - last_point_me)
                    * (cur_split_me - last_point_me);
            let time_val = cur_split_time - prev_split_time;
            let split_dist = if label == "km" {
                cur_point_me / 1000.
            } else {
                cur_point_me / split_distance_in_meters
            };
            let tmp_vector = if do_heart_rate {
                SplitValue {
                    split_distance: split_dist,
                    time_value: time_val,
                    avg_heart_rate: Some(avg_hrt_rate / (cur_split_time - prev_split_time)),
                }
            } else {
                SplitValue {
                    split_distance: split_dist,
                    time_value: time_val,
                    avg_heart_rate: None,
                }
            };

            debug!("get splits 1 {:?}", &tmp_vector);

            split_vector.push(tmp_vector);

            prev_split_time = cur_split_time;
            avg_hrt_rate = 0.0;
        }
        last_point_me = cur_point_me;
        last_point_time = cur_point_time;
    }
    split_vector
}
