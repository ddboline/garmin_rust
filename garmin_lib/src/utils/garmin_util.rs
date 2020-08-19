use anyhow::{format_err, Error};
use chrono::{DateTime, TimeZone, Utc};
use fitparser::Value;
use flate2::{read::GzEncoder, Compression};
use log::{debug, error};
use num_traits::pow::Pow;
use rand::{
    distributions::{Alphanumeric, Distribution, Uniform},
    thread_rng,
};
use stack_string::StackString;
use std::{
    fs::{remove_file, File},
    future::Future,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};
use subprocess::{Exec, Redirection};
use tokio::time::{delay_for, Duration};

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

pub fn convert_xml_local_time_to_utc(xml_local_time: &str) -> Result<DateTime<Utc>, Error> {
    DateTime::parse_from_rfc3339(xml_local_time)
        .map(|x| x.with_timezone(&Utc))
        .map_err(Into::into)
}

pub fn get_md5sum(filename: &Path) -> Result<StackString, Error> {
    let command = format!("md5sum {}", filename.to_string_lossy());

    let stream = Exec::shell(command).stream_stdout()?;

    let reader = BufReader::new(stream);

    if let Some(line) = reader.lines().next() {
        if let Some(entry) = line?.split_whitespace().next() {
            return Ok(entry.into());
        }
    }
    Ok("".into())
}

pub fn print_h_m_s(second: f64, do_hours: bool) -> Result<StackString, Error> {
    let hours = (second / 3600.0) as i32;
    let minutes = (second / 60.0) as i32 - hours * 60;
    let seconds = second as i32 - minutes * 60 - hours * 3600;
    if (hours > 0) | ((hours == 0) & do_hours) {
        Ok(format!("{:02}:{:02}:{:02}", hours, minutes, seconds).into())
    } else if hours == 0 {
        Ok(format!("{:02}:{:02}", minutes, seconds).into())
    } else {
        Err(format_err!("Negative result!"))
    }
}

pub fn days_in_year(year: i32) -> i64 {
    (Utc.ymd(year + 1, 1, 1) - Utc.ymd(year, 1, 1)).num_days()
}

pub fn days_in_month(year: i32, month: u32) -> i64 {
    let mut y1 = year;
    let mut m1 = month + 1;
    if m1 == 13 {
        y1 += 1;
        m1 = 1;
    }
    (Utc.ymd(y1, m1, 1) - Utc.ymd(year, month, 1)).num_days()
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

pub fn titlecase(input: &str) -> StackString {
    if input.is_empty() {
        "".into()
    } else {
        let firstchar = input[0..1].to_uppercase();
        format!("{}{}", firstchar, &input[1..input.len()]).into()
    }
}

pub fn generate_random_string(nchar: usize) -> StackString {
    let mut rng = thread_rng();
    Alphanumeric.sample_iter(&mut rng).take(nchar).collect()
}

pub fn get_file_list(path: &Path) -> Vec<PathBuf> {
    match path.read_dir() {
        Ok(it) => it
            .filter_map(|dir_line| match dir_line {
                Ok(entry) => Some(entry.path()),
                Err(_) => None,
            })
            .collect(),
        Err(err) => {
            debug!("{}", err);
            Vec::new()
        }
    }
}

pub async fn exponential_retry<T, U, F>(f: T) -> Result<U, Error>
where
    T: Fn() -> F,
    F: Future<Output = Result<U, Error>>,
{
    let mut timeout: f64 = 1.0;
    let range = Uniform::from(0..1000);
    loop {
        match f().await {
            Ok(resp) => return Ok(resp),
            Err(err) => {
                delay_for(Duration::from_millis((timeout * 1000.0) as u64)).await;
                timeout *= 4.0 * f64::from(range.sample(&mut thread_rng())) / 1000.0;
                if timeout >= 64.0 {
                    return Err(err);
                }
            }
        }
    }
}

pub fn extract_zip_from_garmin_connect(
    filename: &Path,
    ziptmpdir: &Path,
) -> Result<PathBuf, Error> {
    let new_filename = filename.with_extension("fit");
    let new_filename = new_filename
        .file_name()
        .ok_or_else(|| format_err!("Bad filename"))?;
    let command = format!(
        "unzip {} -d {}",
        filename.to_string_lossy(),
        ziptmpdir.to_string_lossy()
    );
    let mut process = Exec::shell(command).stdout(Redirection::Pipe).popen()?;
    let exit_status = process.wait()?;
    if !exit_status.success() {
        if let Some(mut f) = process.stdout.as_ref() {
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            error!("{}", buf);
        }
        return Err(format_err!("Failed with exit status {:?}", exit_status));
    }
    let new_filename = ziptmpdir.join(new_filename);
    remove_file(filename)?;
    Ok(new_filename)
}

pub fn gzip_file<T, U>(input_filename: T, output_filename: U) -> Result<(), Error>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    let input_filename = input_filename.as_ref();
    let output_filename = output_filename.as_ref();
    if !input_filename.exists() {
        return Err(format_err!("File {:?} does not exist", input_filename));
    }
    std::io::copy(
        &mut GzEncoder::new(File::open(input_filename)?, Compression::fast()),
        &mut File::create(output_filename)?,
    )?;
    Ok(())
}

pub fn get_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Timestamp(val) => Some(val.timestamp() as f64),
        Value::Byte(val) | Value::Enum(val) | Value::UInt8(val) | Value::UInt8z(val) => {
            Some(f64::from(*val))
        }
        Value::SInt8(val) => Some(f64::from(*val)),
        Value::SInt16(val) => Some(f64::from(*val)),
        Value::UInt16(val) | Value::UInt16z(val) => Some(f64::from(*val)),
        Value::SInt32(val) => Some(f64::from(*val)),
        Value::UInt32(val) | Value::UInt32z(val) => Some(f64::from(*val)),
        Value::SInt64(val) => Some(*val as f64),
        Value::UInt64(val) | Value::UInt64z(val) => Some(*val as f64),
        Value::Float32(val) => Some(f64::from(*val)),
        Value::Float64(val) => Some(*val),
        _ => None,
    }
}

pub fn get_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Timestamp(val) => Some(val.timestamp() as i64),
        Value::Byte(val) | Value::Enum(val) | Value::UInt8(val) | Value::UInt8z(val) => {
            Some(i64::from(*val))
        }
        Value::SInt8(val) => Some(i64::from(*val)),
        Value::SInt16(val) => Some(i64::from(*val)),
        Value::UInt16(val) | Value::UInt16z(val) => Some(i64::from(*val)),
        Value::SInt32(val) => Some(i64::from(*val)),
        Value::UInt32(val) | Value::UInt32z(val) => Some(i64::from(*val)),
        Value::SInt64(val) => Some(*val),
        Value::UInt64(val) | Value::UInt64z(val) => Some(*val as i64),
        Value::Float32(val) => Some(*val as i64),
        Value::Float64(val) => Some(*val as i64),
        _ => None,
    }
}

#[inline]
pub fn get_degrees_from_semicircles(s: f64) -> f64 {
    s * 180.0 / (2_147_483_648.0)
}
