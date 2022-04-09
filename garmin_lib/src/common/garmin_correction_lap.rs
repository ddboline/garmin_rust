#![allow(clippy::wrong_self_convention)]

use anyhow::{format_err, Error};
use json::{parse, JsonValue};
use log::debug;
use postgres_query::FromSqlRow;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use stack_string::StackString;
use std::{collections::HashMap, fs, hash::BuildHasher, path::Path, str};
use time::OffsetDateTime;

use crate::utils::{
    garmin_util::METERS_PER_MILE,
    iso_8601_datetime::{convert_str_to_datetime, sentinel_datetime},
    sport_types::SportTypes,
};

use super::{garmin_lap::GarminLap, pgpool::PgPool};

#[derive(Debug, Clone, Copy, PartialEq, FromSqlRow)]
pub struct GarminCorrectionLap {
    pub id: i32,
    pub start_time: OffsetDateTime,
    pub lap_number: i32,
    pub sport: Option<SportTypes>,
    pub distance: Option<f64>,
    pub duration: Option<f64>,
    pub summary_id: Option<i32>,
}

pub type GarminCorrectionMap = HashMap<(OffsetDateTime, i32), GarminCorrectionLap>;

impl Default for GarminCorrectionLap {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminCorrectionLap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: -1,
            start_time: sentinel_datetime(),
            lap_number: -1,
            sport: None,
            distance: None,
            duration: None,
            summary_id: None,
        }
    }

    #[must_use]
    pub fn with_id(mut self, id: i32) -> Self {
        self.id = id;
        self
    }

    #[must_use]
    pub fn with_start_time(mut self, start_time: OffsetDateTime) -> Self {
        self.start_time = start_time;
        self
    }

    #[must_use]
    pub fn with_lap_number(mut self, lap_number: i32) -> Self {
        self.lap_number = lap_number;
        self
    }

    #[must_use]
    pub fn with_sport(mut self, sport: SportTypes) -> Self {
        self.sport = Some(sport);
        self
    }

    #[must_use]
    pub fn with_distance(mut self, distance: f64) -> Self {
        self.distance = Some(distance);
        self
    }

    #[must_use]
    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn map_from_vec<T: IntoIterator<Item = Self>>(corr_list: T) -> GarminCorrectionMap {
        corr_list
            .into_iter()
            .map(|corr| ((corr.start_time, corr.lap_number), corr))
            .collect()
    }

    #[must_use]
    pub fn get_corr_list(corr_map: &GarminCorrectionMap) -> Vec<Self> {
        corr_map.values().copied().collect()
    }

    /// # Errors
    /// Return error if loading correction map fails
    pub fn corr_map_from_buffer(buffer: &[u8]) -> Result<GarminCorrectionMap, Error> {
        let jsval = parse(str::from_utf8(buffer)?)?;

        let corr_map = match &jsval {
            JsonValue::Object(_) => jsval
                .entries()
                .flat_map(|(key, val)| match val {
                    JsonValue::Object(_) => val
                        .entries()
                        .map(|(lap, result)| match result {
                            JsonValue::Number(_) => {
                                let corr = Self::new()
                                    .with_start_time(convert_str_to_datetime(key)?)
                                    .with_lap_number(lap.parse()?);
                                Ok(match result.as_f64() {
                                    Some(r) => corr.with_distance(r),
                                    None => corr,
                                })
                            }
                            JsonValue::Array(arr) => {
                                let corr = Self::new()
                                    .with_start_time(convert_str_to_datetime(key)?)
                                    .with_lap_number(lap.parse()?);
                                let corr = match arr.get(0) {
                                    Some(x) => match x.as_f64() {
                                        Some(r) => corr.with_distance(r),
                                        None => corr,
                                    },
                                    None => corr,
                                };
                                Ok(match arr.get(1) {
                                    Some(x) => match x.as_f64() {
                                        Some(r) => corr.with_duration(r),
                                        None => corr,
                                    },
                                    None => corr,
                                })
                            }
                            _ => Err(format_err!("something unexpected {result}")),
                        })
                        .collect(),
                    _ => Vec::new(),
                })
                .filter_map(|x| match x {
                    Ok(corr) => Some(((corr.start_time, corr.lap_number), corr)),
                    Err(e) => {
                        debug!("Error {}", e);
                        None
                    }
                })
                .collect(),
            _ => HashMap::new(),
        };
        Ok(corr_map)
    }

    /// # Errors
    /// Return error if loading correction map fails
    pub fn corr_list_from_json<T: AsRef<Path>>(
        json_filename: T,
    ) -> Result<GarminCorrectionMap, Error> {
        let buffer = fs::read(json_filename.as_ref())?;
        Self::corr_map_from_buffer(&buffer)
    }

    pub fn add_mislabeled_times_to_corr_list(corr_list_map: &mut GarminCorrectionMap) {
        let mislabeled_times = vec![
            (
                "biking",
                vec![
                    "2010-11-20T19:55:34Z",
                    "2011-05-07T19:43:08Z",
                    "2011-08-29T22:12:18Z",
                    "2011-12-20T18:43:56Z",
                    "2011-08-06T13:59:30Z",
                    "2016-06-30T12:02:39Z",
                ],
            ),
            (
                "running",
                vec![
                    "2010-08-16T22:56:12Z",
                    "2010-08-25T21:52:44Z",
                    "2010-10-31T19:55:51Z",
                    "2011-01-02T21:23:19Z",
                    "2011-05-24T22:13:36Z",
                    "2011-06-27T21:15:29Z",
                    "2012-05-04T21:27:02Z",
                    "2014-02-09T14:26:59Z",
                ],
            ),
            (
                "walking",
                vec![
                    "2012-04-28T15:28:09Z",
                    "2012-05-19T14:35:38Z",
                    "2012-05-19T14:40:29Z",
                    "2012-12-31T20:40:05Z",
                    "2017-04-29T10:04:04Z",
                    "2017-07-01T09:47:14Z",
                ],
            ),
            ("stairs", vec!["2012-02-09T01:43:05Z"]),
            ("snowshoeing", vec!["2013-12-25T19:34:06Z"]),
            (
                "skiing",
                vec![
                    "2010-12-24T19:04:58Z",
                    "2013-12-26T21:24:38Z",
                    "2016-12-30T17:34:03Z",
                ],
            ),
        ];

        for (sport, times_list) in mislabeled_times {
            let sport: SportTypes = sport.parse().unwrap_or(SportTypes::None);
            for time in times_list {
                let time = convert_str_to_datetime(time).expect("Invalid time string");
                let lap_list: Vec<_> = corr_list_map
                    .keys()
                    .filter_map(|(t, n)| if *t == time { Some(*n) } else { None })
                    .collect();

                let lap_list = if lap_list.is_empty() {
                    vec![0]
                } else {
                    lap_list
                };

                for lap_number in lap_list {
                    let new_corr = match corr_list_map.get(&(time, lap_number)) {
                        Some(v) => v.with_sport(sport),
                        None => Self::new()
                            .with_start_time(time)
                            .with_lap_number(lap_number)
                            .with_sport(sport),
                    };

                    corr_list_map.insert((time, lap_number), new_corr);
                }
            }
        }
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn dump_corrections_to_db(
        corr_list_map: &GarminCorrectionMap,
        pool: &PgPool,
    ) -> Result<(), Error> {
        let query_unique_key = "
            SELECT start_time, lap_number
            FROM garmin_corrections_laps
            WHERE start_time=$1 AND lap_number=$2
        ";
        let query_insert = "
            INSERT INTO garmin_corrections_laps
            (start_time, lap_number, distance, duration, sport)
            VALUES
            ($1, $2, $3, $4, $5)
        ";
        let query_update = "
            UPDATE garmin_corrections_laps
            SET distance=$3,duration=$4,sport=$5
            WHERE start_time=$1 AND lap_number=$2
        ";
        let conn = pool.get().await?;
        let stmt_insert = conn.prepare(query_insert).await?;
        let stmt_update = conn.prepare(query_update).await?;
        for corr in corr_list_map.values() {
            let sport: Option<StackString> = corr.sport.and_then(|s| match s {
                SportTypes::None => None,
                s => Some(StackString::from_display(s)),
            });

            if conn
                .query(query_unique_key, &[&corr.start_time, &corr.lap_number])
                .await?
                .is_empty()
            {
                conn.execute(
                    &stmt_insert,
                    &[
                        &corr.start_time,
                        &corr.lap_number,
                        &corr.distance,
                        &corr.duration,
                        &sport,
                    ],
                )
                .await?;
            } else {
                conn.execute(
                    &stmt_update,
                    &[
                        &corr.start_time,
                        &corr.lap_number,
                        &corr.distance,
                        &corr.duration,
                        &sport,
                    ],
                )
                .await?;
            }
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn read_corrections_from_db(pool: &PgPool) -> Result<GarminCorrectionMap, Error> {
        let conn = pool.get().await?;
        conn.query(
            r#"
                    SELECT id, start_time, lap_number, sport, distance, duration, summary_id
                    FROM garmin_corrections_laps
                "#,
            &[],
        )
        .await?
        .par_iter()
        .map(|row| {
            let corr = Self::from_row(row)?;
            Ok(((corr.start_time, corr.lap_number), corr))
        })
        .collect()
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn fix_corrections_in_db(pool: &PgPool) -> Result<(), Error> {
        let query = "
            UPDATE garmin_corrections_laps_backup SET summary_id = (
                SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time
            )
            WHERE summary_id IS NULL
        ";
        pool.get()
            .await?
            .execute(query, &[])
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn apply_lap_corrections<S: BuildHasher + Sync>(
    lap_list: &[GarminLap],
    sport: SportTypes,
    corr_map: &HashMap<(OffsetDateTime, i32), GarminCorrectionLap, S>,
) -> (Vec<GarminLap>, SportTypes) {
    let mut new_sport = sport;
    match lap_list.get(0) {
        Some(l) => {
            let lap_start = l.lap_start;
            for lap in lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            let new_lap_list: Vec<_> = lap_list
                .iter()
                .map(|lap| {
                    let lap_number = lap.lap_number;
                    match &corr_map.get(&(lap_start, lap_number)) {
                        Some(corr) => {
                            let mut new_lap = lap.clone();
                            match corr.sport {
                                None | Some(SportTypes::None) => {}
                                Some(s) => {
                                    debug!("change sport {} {:?} {}", lap_start, lap.lap_type, s);
                                    new_sport = s;
                                }
                            };
                            new_lap.lap_duration = match &corr.duration {
                                Some(dur) => {
                                    debug!(
                                        "change duration {} {} {}",
                                        lap_start, lap.lap_duration, dur
                                    );
                                    *dur
                                }
                                None => lap.lap_duration,
                            };
                            new_lap.lap_distance = match &corr.distance {
                                Some(dis) => {
                                    debug!(
                                        "change duration {} {} {}",
                                        lap_start,
                                        lap.lap_distance,
                                        dis * METERS_PER_MILE
                                    );
                                    dis * METERS_PER_MILE
                                }
                                None => lap.lap_distance,
                            };
                            new_lap
                        }
                        None => lap.clone(),
                    }
                })
                .collect();
            for lap in &new_lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            (new_lap_list, new_sport)
        }
        None => (Vec::new(), new_sport),
    }
}
