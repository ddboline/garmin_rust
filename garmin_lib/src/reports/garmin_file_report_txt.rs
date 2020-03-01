use anyhow::Error;
use log::debug;

use crate::{
    common::{garmin_file::GarminFile, garmin_lap::GarminLap},
    utils::{
        garmin_util::{print_h_m_s, MARATHON_DISTANCE_MI, METERS_PER_MILE},
        sport_types::SportTypes,
    },
};

pub fn generate_txt_report(gfile: &GarminFile) -> Result<Vec<String>, Error> {
    let mut return_vec = vec![format!("Start time {}", gfile.filename)];

    let sport_type = gfile.sport;

    for lap in &gfile.laps {
        return_vec.push(print_lap_string(&lap, sport_type)?)
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
    if sport_type == SportTypes::Running {
        tmp_str.push(format!(
            "total {:.2} mi {} calories {} time {} min/mi {} min/km",
            gfile.total_distance / METERS_PER_MILE,
            gfile.total_calories,
            print_h_m_s(gfile.total_duration, true)?,
            print_h_m_s(min_mile * 60.0, false)?,
            print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false)?
        ));
    } else {
        tmp_str.push(format!(
            "total {:.2} mi {} calories {} time {} mph",
            gfile.total_distance / METERS_PER_MILE,
            gfile.total_calories,
            print_h_m_s(gfile.total_duration, true)?,
            format!("{:.2}", mi_per_hr),
        ));
    }

    if gfile.total_hr_dur > gfile.total_hr_dis {
        tmp_str.push(format!(
            "{:.2} bpm",
            (gfile.total_hr_dur / gfile.total_hr_dis) as i32
        ));
    };
    return_vec.push(tmp_str.join(" "));
    return_vec.push("".to_string());
    return_vec.push(print_splits(&gfile, METERS_PER_MILE, "mi")?);
    return_vec.push("".to_string());
    return_vec.push(print_splits(&gfile, 5000.0, "km")?);

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

    if (sum_time > 0.0) & !hr_vals.is_empty() {
        return_vec.push("".to_string());
        return_vec.push(format!(
            "Heart Rate {:2.2} avg {:2.2} max",
            avg_hr,
            hr_vals.iter().map(|x| *x as i32).max().unwrap_or(0)
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

    if !alt_vals.is_empty() {
        return_vec.push(format!(
            "max altitude diff: {:.2} m",
            alt_vals.iter().map(|x| *x as i32).max().unwrap_or(0)
                - alt_vals.iter().map(|x| *x as i32).min().unwrap_or(0)
        ));
        return_vec.push(format!("vertical climb: {:.2} m", vertical_climb));
    }

    Ok(return_vec)
}

fn print_lap_string(glap: &GarminLap, sport: SportTypes) -> Result<String, Error> {
    let sport_str = sport.to_string();

    let mut outstr = vec![format!(
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
        outstr.push("/ mi ".to_string());
        outstr.push(print_h_m_s(
            glap.lap_duration / (glap.lap_distance / 1000.),
            false,
        )?);
        outstr.push("/ km".to_string());
    };
    if let Some(x) = glap.lap_avg_hr {
        if x > 0.0 {
            outstr.push(format!("{} bpm", x));
        }
    }

    Ok(outstr.join(" "))
}

fn print_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
) -> Result<String, Error> {
    if gfile.points.is_empty() {
        return Ok("".to_string());
    }

    let retval: Vec<_> = get_splits(gfile, split_distance_in_meters, label, true)?
        .into_iter()
        .map(|val| {
            let dis = val.split_distance as i32;
            let tim = val.time_value;
            let hrt = val.avg_heart_rate.unwrap_or(0.0);

            format!(
                "{} {} \t {} \t {} / mi \t {} / km \t {} \t {:.2} bpm avg",
                dis,
                label,
                print_h_m_s(tim, true).unwrap_or_else(|_| "".to_string()),
                print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false)
                    .unwrap_or_else(|_| "".to_string()),
                print_h_m_s(tim / (split_distance_in_meters / 1000.), false)
                    .unwrap_or_else(|_| "".to_string()),
                print_h_m_s(
                    tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                    true
                )
                .unwrap_or_else(|_| "".to_string()),
                hrt
            )
        })
        .collect();
    Ok(retval.join("\n"))
}

#[derive(Debug)]
pub struct SplitValue {
    pub split_distance: f64,
    pub time_value: f64,
    pub avg_heart_rate: Option<f64>,
}

pub fn get_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
    do_heart_rate: bool,
) -> Result<Vec<SplitValue>, Error> {
    if gfile.points.len() < 3 {
        return Ok(Vec::new());
    };
    let mut last_point_me = 0.0;
    let mut last_point_time = 0.0;
    let mut prev_split_time = 0.0;
    let mut avg_hrt_rate = 0.0;

    let mut split_vector = Vec::new();

    for point in &gfile.points {
        let cur_point_me = match point.distance {
            Some(x) => x,
            None => continue,
        };
        let cur_point_time = point.duration_from_begin;
        if (cur_point_me - last_point_me) <= 0.0 {
            continue;
        }

        if let Some(hr) = point.heart_rate {
            avg_hrt_rate += hr * (cur_point_time - last_point_time)
        };

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
        };
        last_point_me = cur_point_me;
        last_point_time = cur_point_time;
    }
    Ok(split_vector)
}
