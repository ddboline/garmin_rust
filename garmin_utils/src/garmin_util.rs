use anyhow::{format_err, Error};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use fitparser::Value;
use flate2::{read::GzEncoder, Compression};
use futures::{Stream, TryStreamExt};
use log::debug;
use num_traits::pow::Pow;
use postgres_query::{query, Error as PqError, FromSqlRow};
use rand::{
    distributions::{Alphanumeric, Distribution, Uniform},
    thread_rng, Rng,
};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    fs::{remove_file, File},
    future::Future,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use subprocess::Exec;
use time::{format_description::well_known::Rfc3339, macros::date, Date, Month, OffsetDateTime};
use time_tz::{timezones::db::UTC, OffsetDateTimeExt};
use tokio::time::{sleep, Duration};
use zip::ZipArchive;

use crate::pgpool::PgPool;

pub const METERS_PER_MILE: f64 = 1609.344;
pub const MARATHON_DISTANCE_M: i32 = 42195;
pub const MARATHON_DISTANCE_MI: f64 = MARATHON_DISTANCE_M as f64 / METERS_PER_MILE;

pub const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// # Errors
/// Return error if parsing time string fails
pub fn convert_time_string(time_str: &str) -> Result<f64, Error> {
    let entries: SmallVec<[&str; 3]> = time_str.split(':').take(3).collect();
    let (h, m, s): (i32, i32, f64) = match entries.first() {
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

/// # Errors
/// Return error if parsing time string fails
pub fn convert_xml_local_time_to_utc(xml_local_time: &str) -> Result<OffsetDateTime, Error> {
    OffsetDateTime::parse(xml_local_time, &Rfc3339)
        .map(|x| x.to_timezone(UTC))
        .map_err(Into::into)
}

/// # Errors
/// Return error if running `md5sum` fails
pub fn get_md5sum(filename: &Path) -> Result<StackString, Error> {
    if !Path::new("/usr/bin/md5sum").exists() {
        return Err(format_err!(
            "md5sum not installed (or not present at /usr/bin/md5sum)"
        ));
    }
    let command = format_sstr!("md5sum {}", filename.to_string_lossy());

    let stream = Exec::shell(command).stream_stdout()?;

    let reader = BufReader::new(stream);

    if let Some(line) = reader.lines().next() {
        if let Some(entry) = line?.split_whitespace().next() {
            return Ok(entry.into());
        }
    }
    Ok("".into())
}

/// # Errors
/// Return error if second is negative
pub fn print_h_m_s(second: f64, do_hours: bool) -> Result<StackString, Error> {
    if second.abs() > (i64::MAX as f64) {
        return Err(format_err!("Number of seconds is far too large"));
    }
    let hours = (second / 3600.0) as i64;
    let minutes = (second / 60.0) as i64 - hours * 60;
    let seconds = second as i64 - minutes * 60 - hours * 3600;
    if (hours > 0) | ((hours == 0) & do_hours) {
        Ok(format_sstr!("{hours:02}:{minutes:02}:{seconds:02}"))
    } else if hours == 0 {
        Ok(format_sstr!("{minutes:02}:{seconds:02}"))
    } else {
        Err(format_err!("Negative result!"))
    }
}

#[must_use]
pub fn days_in_year(year: i32) -> i64 {
    (Date::from_calendar_date(year + 1, Month::January, 1).unwrap_or(date!(1969 - 01 - 01))
        - Date::from_calendar_date(year, Month::January, 1).unwrap_or(date!(1969 - 01 - 01)))
    .whole_days()
}

#[must_use]
pub fn days_in_month(year: i32, month: u32) -> i64 {
    let mut y1 = year;
    let mut m1 = month + 1;
    if m1 == 13 {
        y1 += 1;
        m1 = 1;
    }
    let month: Month = (month as u8).try_into().unwrap_or(Month::January);
    let m1: Month = (m1 as u8).try_into().unwrap_or(Month::January);
    (Date::from_calendar_date(y1, m1, 1).unwrap_or(date!(1969 - 01 - 01))
        - Date::from_calendar_date(year, month, 1).unwrap_or(date!(1969 - 01 - 01)))
    .whole_days()
}

#[must_use]
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

#[must_use]
pub fn titlecase(input: &str) -> StackString {
    if input.is_empty() {
        "".into()
    } else {
        let firstchar = input[0..1].to_uppercase();
        format_sstr!("{firstchar}{s}", s = &input[1..])
    }
}

#[must_use]
pub fn generate_random_string(nchar: usize) -> StackString {
    let mut rng = thread_rng();
    Alphanumeric
        .sample_iter(&mut rng)
        .take(nchar)
        .map(Into::into)
        .collect()
}

#[must_use]
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

/// # Errors
/// Return error if closure fails
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
                sleep(Duration::from_millis((timeout * 1000.0) as u64)).await;
                timeout *= 4.0 * f64::from(range.sample(&mut thread_rng())) / 1000.0;
                if timeout >= 64.0 {
                    return Err(err);
                }
            }
        }
    }
}

fn extract_zip(filename: &Path, ziptmpdir: &Path) -> Result<Vec<PathBuf>, Error> {
    if !Path::new("/usr/bin/unzip").exists() {
        return Err(format_err!(
            "md5sum not installed (or not present at /usr/bin/unzip"
        ));
    }
    let mut zip = ZipArchive::new(File::open(filename)?)?;
    (0..zip.len())
        .map(|i| {
            let mut f = zip.by_index(i)?;
            let fpath = ziptmpdir.join(f.name());
            let mut g = File::create(&fpath)?;
            std::io::copy(&mut f, &mut g)?;
            Ok(fpath)
        })
        .collect()
}

/// # Errors
/// Return error if unzip fails
pub fn extract_zip_from_garmin_connect(
    filename: &Path,
    ziptmpdir: &Path,
) -> Result<PathBuf, Error> {
    extract_zip(filename, ziptmpdir)?;
    let new_filename = filename
        .file_stem()
        .ok_or_else(|| format_err!("Bad filename {}", filename.to_string_lossy()))?;
    let new_filename = format_sstr!("{}_ACTIVITY.fit", new_filename.to_string_lossy());
    let new_filename = ziptmpdir.join(new_filename);
    if !new_filename.exists() {
        return Err(format_err!("Activity file not found"));
    }
    remove_file(filename)?;
    Ok(new_filename)
}

/// # Errors
/// Return error if unzip fails
pub fn extract_zip_from_garmin_connect_multiple(
    filename: &Path,
    ziptmpdir: &Path,
) -> Result<Vec<PathBuf>, Error> {
    extract_zip(filename, ziptmpdir)
}

/// # Errors
/// Return error if:
///     * input file does not exist
///     * opening it fails
///     * creating the output file fails
///     * writing to the file fails
pub fn gzip_file<T, U>(input_filename: T, output_filename: U) -> Result<(), Error>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    let input_filename = input_filename.as_ref();
    let output_filename = output_filename.as_ref();
    if !input_filename.exists() {
        return Err(format_err!("File {input_filename:?} does not exist"));
    }
    std::io::copy(
        &mut GzEncoder::new(File::open(input_filename)?, Compression::fast()),
        &mut File::create(output_filename)?,
    )?;
    Ok(())
}

#[must_use]
pub fn get_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Timestamp(val) => Some(val.unix_timestamp() as f64),
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

#[must_use]
pub fn get_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Timestamp(val) => Some(val.unix_timestamp()),
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
#[must_use]
pub fn get_degrees_from_semicircles(s: f64) -> f64 {
    s * 180.0 / (2_147_483_648.0)
}

#[derive(FromSqlRow, Clone, Debug)]
pub struct AuthorizedUsers {
    pub email: StackString,
    pub telegram_userid: Option<i64>,
    pub created_at: OffsetDateTime,
}

impl AuthorizedUsers {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_authorized_users(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!("SELECT * FROM authorized_users WHERE deleted_at IS NULL");
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_most_recent(
        pool: &PgPool,
    ) -> Result<(Option<OffsetDateTime>, Option<OffsetDateTime>), Error> {
        #[derive(FromSqlRow)]
        struct CreatedDeleted {
            created_at: Option<OffsetDateTime>,
            deleted_at: Option<OffsetDateTime>,
        }

        let query = query!(
            "SELECT max(created_at) as created_at, max(deleted_at) as deleted_at FROM \
             authorized_users"
        );
        let conn = pool.get().await?;
        let result: Option<CreatedDeleted> = query.fetch_opt(&conn).await?;
        match result {
            Some(result) => Ok((result.created_at, result.deleted_at)),
            None => Ok((None, None)),
        }
    }
}

/// # Errors
/// Return error if db query fails
pub async fn get_list_of_telegram_userids(
    pool: &PgPool,
) -> Result<impl Stream<Item = Result<i64, PqError>>, Error> {
    let query = query!(
        "
    SELECT distinct telegram_userid
    FROM authorized_users
    WHERE telegram_userid IS NOT NULL
      AND deleted_at IS NULL
    "
    );
    let conn = pool.get().await?;
    query
        .query_streaming(&conn)
        .await
        .map(|stream| {
            stream.and_then(|row| async move {
                let telegram_id: i64 = row
                    .try_get("telegram_userid")
                    .map_err(PqError::BeginTransaction)?;
                Ok(telegram_id)
            })
        })
        .map_err(Into::into)
}

#[must_use]
pub fn get_random_string() -> StackString {
    let random_bytes: SmallVec<[u8; 16]> = (0..16).map(|_| thread_rng().gen::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&random_bytes).into()
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use std::path::Path;
    use tempfile::TempDir;

    use crate::garmin_util::extract_zip;

    #[test]
    fn test_extract_zip() -> Result<(), Error> {
        let d = TempDir::with_prefix("garmin_util")?;
        let p = d.path();

        let zip_path = Path::new("../tests/data/test.zip");
        assert!(zip_path.exists());
        let files = extract_zip(zip_path, p)?;
        assert!(files.len() == 2);
        Ok(())
    }
}
