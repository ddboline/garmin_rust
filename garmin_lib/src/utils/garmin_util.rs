use num::traits::Pow;

use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

use rand::distributions::{Alphanumeric, Distribution, Uniform};
use rand::thread_rng;
use std::thread::sleep;
use std::time::Duration;

use chrono::prelude::*;

use failure::{err_msg, Error};

pub const METERS_PER_MILE: f64 = 1609.344;
pub const MARATHON_DISTANCE_M: i32 = 42195;
pub const MARATHON_DISTANCE_MI: f64 = MARATHON_DISTANCE_M as f64 / METERS_PER_MILE;

pub const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn convert_time_string(time_str: &str) -> Result<f64, Error> {
    let entries: Vec<_> = time_str.split(':').collect();
    let (h, m, s): (i32, i32, f64) = match entries.get(0) {
        Some(h) => match entries.get(1) {
            Some(m) => match entries.get(2) {
                Some(s) => (h.parse()?, m.parse()?, s.parse()?),
                None => (h.parse()?, m.parse()?, 0.),
            },
            None => (h.parse()?, 0, 0.),
        },
        None => (0, 0, 0.),
    };
    Ok(s + 60.0 * (f64::from(m) + 60.0 * f64::from(h)))
}

pub fn convert_xml_local_time_to_utc(xml_local_time: &str) -> Result<String, Error> {
    let local = DateTime::parse_from_rfc3339(xml_local_time)?.with_timezone(&FixedOffset::east(0));
    Ok(local.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

pub fn get_md5sum(filename: &str) -> Result<String, Error> {
    let command = format!("md5sum {}", filename);

    let stream = Exec::shell(command).stream_stdout()?;

    let reader = BufReader::new(stream);

    if let Some(line) = reader.lines().next() {
        if let Some(entry) = line?.split_whitespace().next() {
            Ok(entry.to_string())
        } else {
            Ok("".to_string())
        }
    } else {
        Ok("".to_string())
    }
}

pub fn print_h_m_s(second: f64, do_hours: bool) -> Result<String, Error> {
    let hours = (second / 3600.0) as i32;
    let minutes = (second / 60.0) as i32 - hours * 60;
    let seconds = second as i32 - minutes * 60 - hours * 3600;
    if (hours > 0) | ((hours == 0) & do_hours) {
        Ok(format!("{:02}:{:02}:{:02}", hours, minutes, seconds))
    } else if hours == 0 {
        Ok(format!("{:02}:{:02}", minutes, seconds))
    } else {
        Err(err_msg("Negative result!"))
    }
}

pub fn days_in_year(year: i32) -> i64 {
    (Utc.ymd(year + 1, 1, 1) - Utc.ymd(year, 1, 1)).num_days()
}

pub fn days_in_month(year: i32, month: u32) -> i64 {
    let mut y1_ = year;
    let mut m1_ = month + 1;
    if m1_ == 13 {
        y1_ += 1;
        m1_ = 1;
    }
    (Utc.ymd(y1_, m1_, 1) - Utc.ymd(year, month, 1)).num_days()
}

pub fn expected_calories(weight: f64, pace_min_per_mile: f64, distance: f64) -> f64 {
    let cal_per_mi = weight
        * (0.0395
            + 0.003_27 * (60. / pace_min_per_mile)
            + 0.000_455 * (60. / pace_min_per_mile).pow(2.0)
            + 0.000_801
                * ((weight / 154.0) * 0.425 / weight * (60. / pace_min_per_mile).pow(3.0))
                * 60.
                / (60. / pace_min_per_mile));
    cal_per_mi * distance
}

pub fn titlecase(input: &str) -> String {
    if input.is_empty() {
        "".to_string()
    } else {
        let firstchar = input[0..1].to_uppercase();
        format!("{}{}", firstchar, &input[1..input.len()])
    }
}

pub fn map_result_vec<T>(input: Vec<Result<T, Error>>) -> Result<Vec<T>, Error> {
    let mut output: Vec<T> = Vec::new();
    let mut errors: Vec<_> = Vec::new();
    for item in input {
        match item {
            Ok(i) => output.push(i),
            Err(e) => errors.push(format!("{}", e)),
        }
    }
    if !errors.is_empty() {
        Err(err_msg(errors.join("\n")))
    } else {
        Ok(output)
    }
}

pub fn generate_random_string(nchar: usize) -> String {
    let mut rng = thread_rng();
    Alphanumeric.sample_iter(&mut rng).take(nchar).collect()
}

pub fn get_file_list(path: &Path) -> Vec<String> {
    match path.read_dir() {
        Ok(it) => it
            .filter_map(|dir_line| match dir_line {
                Ok(entry) => entry.path().to_str().map(|x| x.to_string()),
                Err(_) => None,
            })
            .collect(),
        Err(err) => {
            println!("{}", err);
            Vec::new()
        }
    }
}

pub fn exponential_retry<T, U>(closure: T) -> Result<U, Error>
where
    T: Fn() -> Result<U, Error>,
{
    let mut timeout: f64 = 1.0;
    let mut rng = thread_rng();
    let range = Uniform::from(0..1000);
    loop {
        match closure() {
            Ok(x) => return Ok(x),
            Err(e) => {
                sleep(Duration::from_millis((timeout * 1000.0) as u64));
                timeout *= 4.0 * f64::from(range.sample(&mut rng)) / 1000.0;
                if timeout >= 64.0 {
                    return Err(err_msg(e));
                }
            }
        }
    }
}