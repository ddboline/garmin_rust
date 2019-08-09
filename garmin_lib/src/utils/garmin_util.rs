use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use failure::{err_msg, Error};
use num::traits::Pow;
use rand::distributions::{Alphanumeric, Distribution, Uniform};
use rand::thread_rng;
use std::io::{stdout, BufRead, BufReader, Read, Write};
use std::iter::FromIterator;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;
use subprocess::{Exec, Redirection};

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

pub fn map_result<T, U, V>(input: U) -> Result<V, Error>
where
    U: IntoIterator<Item = Result<T, Error>>,
    V: FromIterator<T>,
{
    let (output, errors): (Vec<_>, Vec<_>) = input.into_iter().partition(Result::is_ok);
    if !errors.is_empty() {
        let errors: Vec<_> = errors
            .into_iter()
            .filter_map(Result::err)
            .map(|x| x.to_string())
            .collect();
        Err(err_msg(errors.join("\n")))
    } else {
        Ok(output.into_iter().filter_map(Result::ok).collect())
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
                writeln!(stdout().lock(), "Got Error {:?} , retrying", e)?;
                sleep(Duration::from_millis((timeout * 1000.0) as u64));
                timeout *= 4.0 * f64::from(range.sample(&mut rng)) / 1000.0;
                if timeout >= 64.0 {
                    return Err(err_msg(e));
                }
            }
        }
    }
}

pub fn extract_zip_from_garmin_connect(filename: &str, ziptmpdir: &str) -> Result<String, Error> {
    let new_filename = Path::new(filename)
        .file_name()
        .ok_or_else(|| err_msg("Bad filename"))?
        .to_str()
        .ok_or_else(|| err_msg("Bad string"))?;
    let new_filename = new_filename.replace(".zip", ".fit");
    let command = format!("unzip {} -d {}", filename, ziptmpdir);
    let mut process = Exec::shell(command).stdout(Redirection::Pipe).popen()?;
    let exit_status = process.wait()?;
    if !exit_status.success() {
        if let Some(mut f) = process.stdout.as_ref() {
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            writeln!(stdout().lock(), "{}", buf)?;
        }
        return Err(err_msg(format!(
            "Failed with exit status {:?}",
            exit_status
        )));
    }
    let new_filename = format!("{}/{}", ziptmpdir, new_filename);
    Ok(new_filename)
}

pub fn gzip_file(input_filename: &str, output_filename: &str) -> Result<(), Error> {
    let command = format!("gzip -c {} > {}", input_filename, output_filename);
    let mut process = Exec::shell(command).stdout(Redirection::Pipe).popen()?;
    let exit_status = process.wait()?;
    if !exit_status.success() {
        if let Some(mut f) = process.stdout.as_ref() {
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            writeln!(stdout().lock(), "{}", buf)?;
        }
        return Err(err_msg(format!(
            "Failed with exit status {:?}",
            exit_status
        )));
    }
    Ok(())
}
