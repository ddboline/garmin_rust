#![allow(clippy::wrong_self_convention)]

use futures::{Stream, TryStreamExt};
use json::{parse, JsonValue};
use log::debug;
use postgres_query::{query, Error as PqError, FromSqlRow};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fs, hash::BuildHasher, path::Path, str};
use uuid::Uuid;

use garmin_lib::{
    date_time_wrapper::{iso8601::convert_str_to_datetime, DateTimeWrapper},
    errors::GarminError as Error,
};

use garmin_utils::{garmin_util::METERS_PER_MILE, pgpool::PgPool, sport_types::SportTypes};

use crate::garmin_lap::GarminLap;

#[derive(Debug, Clone, Copy, PartialEq, FromSqlRow)]
pub struct GarminCorrectionLap {
    pub id: Uuid,
    pub start_time: DateTimeWrapper,
    pub lap_number: i32,
    pub sport: Option<SportTypes>,
    pub distance: Option<f64>,
    pub duration: Option<f64>,
    pub summary_id: Option<Uuid>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CorrectionKey {
    pub datetime: DateTimeWrapper,
    pub lap_number: i32,
}

pub type GarminCorrectionMap = HashMap<CorrectionKey, GarminCorrectionLap>;

impl Default for GarminCorrectionLap {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminCorrectionLap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            start_time: DateTimeWrapper::sentinel_datetime(),
            lap_number: -1,
            sport: None,
            distance: None,
            duration: None,
            summary_id: None,
        }
    }

    #[must_use]
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    #[must_use]
    pub fn with_start_time(mut self, start_time: DateTimeWrapper) -> Self {
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
        let mut h: HashMap<_, _> = corr_list
            .into_iter()
            .map(|corr| {
                (
                    CorrectionKey {
                        datetime: corr.start_time,
                        lap_number: corr.lap_number,
                    },
                    corr,
                )
            })
            .collect();
        h.shrink_to_fit();
        h
    }

    #[must_use]
    pub fn get_corr_list(corr_map: &GarminCorrectionMap) -> Vec<Self> {
        let mut c: Vec<_> = corr_map.values().copied().collect();
        c.shrink_to_fit();
        c
    }

    /// # Errors
    /// Return error if loading correction map fails
    pub fn corr_map_from_buffer(buffer: &[u8]) -> Result<GarminCorrectionMap, Error> {
        let jsval = parse(str::from_utf8(buffer)?)?;

        let mut corr_map = match &jsval {
            JsonValue::Object(_) => jsval
                .entries()
                .flat_map(|(key, val)| match val {
                    JsonValue::Object(_) => val
                        .entries()
                        .map(|(lap, result)| match result {
                            JsonValue::Number(_) => {
                                let corr = Self::new()
                                    .with_start_time(convert_str_to_datetime(key)?.into())
                                    .with_lap_number(lap.parse()?);
                                Ok(match result.as_f64() {
                                    Some(r) => corr.with_distance(r),
                                    None => corr,
                                })
                            }
                            JsonValue::Array(arr) => {
                                let corr = Self::new()
                                    .with_start_time(convert_str_to_datetime(key)?.into())
                                    .with_lap_number(lap.parse()?);
                                let corr = match arr.first() {
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
                            _ => Err(Error::CustomError(format_sstr!(
                                "something unexpected {result}"
                            ))),
                        })
                        .collect(),
                    _ => Vec::new(),
                })
                .filter_map(|x| match x {
                    Ok(corr) => Some((
                        CorrectionKey {
                            datetime: corr.start_time,
                            lap_number: corr.lap_number,
                        },
                        corr,
                    )),
                    Err(e) => {
                        debug!("Error {e}",);
                        None
                    }
                })
                .collect(),
            _ => HashMap::new(),
        };
        corr_map.shrink_to_fit();
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

    /// # Errors
    /// Returns error if any timestamps are invalid
    pub fn add_mislabeled_times_to_corr_list(
        corr_list_map: &mut GarminCorrectionMap,
    ) -> Result<(), Error> {
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
                let time = convert_str_to_datetime(time)?.into();
                let mut lap_list: Vec<_> = corr_list_map
                    .keys()
                    .filter_map(
                        |CorrectionKey {
                             datetime: t,
                             lap_number: n,
                         }| if *t == time { Some(*n) } else { None },
                    )
                    .collect();
                lap_list.shrink_to_fit();

                let lap_list = if lap_list.is_empty() {
                    vec![0]
                } else {
                    lap_list
                };

                for lap_number in lap_list {
                    let key = CorrectionKey {
                        datetime: time,
                        lap_number,
                    };
                    let new_corr = match corr_list_map.get(&key) {
                        Some(v) => v.with_sport(sport),
                        None => Self::new()
                            .with_start_time(time)
                            .with_lap_number(lap_number)
                            .with_sport(sport),
                    };

                    corr_list_map.insert(key, new_corr);
                }
            }
        }
        Ok(())
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
        Self::_read_corrections_from_db(pool)
            .await?
            .map_ok(|corr| {
                (
                    CorrectionKey {
                        datetime: corr.start_time,
                        lap_number: corr.lap_number,
                    },
                    corr,
                )
            })
            .try_collect()
            .await
            .map_err(Into::into)
    }

    async fn _read_corrections_from_db(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<GarminCorrectionLap, PqError>>, Error> {
        let query = query!(
            r#"
                SELECT id, start_time, lap_number, sport, distance, duration, summary_id
                FROM garmin_corrections_laps
            "#
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn fix_corrections_in_db(pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
            UPDATE garmin_corrections_laps SET summary_id = (
                SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time
            )
            WHERE summary_id IS NULL
        "
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into).map(|_| ())
    }
}

pub struct CorrectedOutput {
    pub laps: Vec<GarminLap>,
    pub sport: SportTypes,
}

pub fn apply_lap_corrections<S: BuildHasher + Sync>(
    lap_list: &[GarminLap],
    sport: SportTypes,
    corr_map: &HashMap<CorrectionKey, GarminCorrectionLap, S>,
) -> CorrectedOutput {
    let mut new_sport = sport;
    match lap_list.first() {
        Some(l) => {
            let lap_start = l.lap_start;
            for lap in lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            let mut new_lap_list: Vec<_> = lap_list
                .iter()
                .map(|lap| {
                    let lap_number = lap.lap_number;
                    let key = CorrectionKey {
                        datetime: lap_start,
                        lap_number,
                    };
                    match &corr_map.get(&key) {
                        Some(corr) => {
                            let mut new_lap = lap.clone();
                            match corr.sport {
                                None | Some(SportTypes::None) => {}
                                Some(s) => {
                                    debug!("change sport {} {:?} {}", lap_start, lap.lap_type, s);
                                    new_sport = s;
                                }
                            }
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
            new_lap_list.shrink_to_fit();
            for lap in &new_lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            CorrectedOutput {
                laps: new_lap_list,
                sport: new_sport,
            }
        }
        None => CorrectedOutput {
            laps: Vec::new(),
            sport: new_sport,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::{stdout, Write};
    use uuid::Uuid;

    use garmin_lib::{
        date_time_wrapper::iso8601::convert_str_to_datetime, errors::GarminError as Error,
    };

    use garmin_utils::sport_types::SportTypes;

    use crate::garmin_correction_lap::{CorrectionKey, GarminCorrectionLap};

    #[test]
    fn test_garmin_correction_lap_new() {
        let gc = GarminCorrectionLap::new();

        assert_eq!(gc.lap_number, -1);
        assert_eq!(gc.sport, None);
        assert_eq!(gc.distance, None);
        assert_eq!(gc.duration, None);

        let new_uuid = Uuid::new_v4();

        let gc = GarminCorrectionLap::new()
            .with_id(new_uuid)
            .with_lap_number(3)
            .with_sport(SportTypes::Running)
            .with_distance(5.3)
            .with_duration(6.2);
        assert_eq!(gc.id, new_uuid);
        assert_eq!(gc.lap_number, 3);
        assert_eq!(gc.sport, Some(SportTypes::Running));
        assert_eq!(gc.distance, Some(5.3));
        assert_eq!(gc.duration, Some(6.2));
    }

    #[test]
    fn test_corr_list_from_json() -> Result<(), Error> {
        let mut corr_list: Vec<_> =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap()
                .into_iter()
                .map(|(_, v)| v)
                .collect();
        corr_list.shrink_to_fit();
        corr_list.sort_by_key(|i| (i.start_time, i.lap_number));

        assert_eq!(corr_list.first().unwrap().distance, Some(3.10685596118667));

        let corr_val = GarminCorrectionLap::new();
        assert_eq!(corr_val.lap_number, -1);
        Ok(())
    }

    #[test]
    fn test_corr_map_from_buffer() -> Result<(), Error> {
        let json_buffer = r#"
            {
                "2011-07-04T08:58:27Z": {
                "0": 3.10685596118667
                },
                "2013-01-17T16:14:32Z": {
                "0": 0.507143,
                "1": 0.190476
                },
                "2014-08-23T10:17:14Z": {
                "0": [
                6.5,
                4099.0
                ]
                },
                "abcdefg": {"hijk": [0, 1, 2]}
            }
            "#
        .to_string()
        .into_bytes();

        let mut corr_list: Vec<_> = GarminCorrectionLap::corr_map_from_buffer(&json_buffer)
            .unwrap()
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        corr_list.shrink_to_fit();
        corr_list.sort_by_key(|i| (i.start_time, i.lap_number));

        let first = corr_list.first().unwrap();
        let second = corr_list.get(1).unwrap();
        let third = corr_list.get(2).unwrap();
        let fourth = corr_list.get(3).unwrap();
        assert_eq!(corr_list.get(4), None);

        assert_eq!(
            first,
            &GarminCorrectionLap {
                id: first.id,
                start_time: convert_str_to_datetime("2011-07-04T08:58:27Z")
                    .unwrap()
                    .into(),
                lap_number: 0,
                sport: None,
                distance: Some(3.10685596118667),
                duration: None,
                ..GarminCorrectionLap::default()
            }
        );
        assert_eq!(
            second,
            &GarminCorrectionLap {
                id: second.id,
                start_time: convert_str_to_datetime("2013-01-17T16:14:32Z")
                    .unwrap()
                    .into(),
                lap_number: 0,
                sport: None,
                distance: Some(0.507143),
                duration: None,
                ..GarminCorrectionLap::default()
            }
        );
        assert_eq!(
            third,
            &GarminCorrectionLap {
                id: third.id,
                start_time: convert_str_to_datetime("2013-01-17T16:14:32Z")
                    .unwrap()
                    .into(),
                lap_number: 1,
                sport: None,
                distance: Some(0.190476),
                duration: None,
                ..GarminCorrectionLap::default()
            }
        );
        assert_eq!(
            fourth,
            &GarminCorrectionLap {
                id: fourth.id,
                start_time: convert_str_to_datetime("2014-08-23T10:17:14Z")
                    .unwrap()
                    .into(),
                lap_number: 0,
                sport: None,
                distance: Some(6.5),
                duration: Some(4099.0),
                ..GarminCorrectionLap::default()
            }
        );
        Ok(())
    }

    #[test]
    fn test_corr_map_from_buffer_invalid() -> Result<(), Error> {
        let json_buffer = r#"["a", "b", "c"]"#.to_string().into_bytes();

        let corr_map = GarminCorrectionLap::corr_map_from_buffer(&json_buffer).unwrap();

        assert_eq!(corr_map.len(), 0);
        Ok(())
    }

    #[test]
    fn test_add_mislabeled_times_to_corr_list() -> Result<(), Error> {
        let id_0 = Uuid::new_v4();
        let id_1 = Uuid::new_v4();
        let mut corr_map = GarminCorrectionLap::map_from_vec(vec![
            GarminCorrectionLap::new()
                .with_id(id_0)
                .with_start_time(
                    convert_str_to_datetime("2010-11-20T19:55:34Z")
                        .unwrap()
                        .into(),
                )
                .with_distance(10.0)
                .with_lap_number(0),
            GarminCorrectionLap::new()
                .with_id(id_1)
                .with_start_time(
                    convert_str_to_datetime("2010-11-20T19:55:34Z")
                        .unwrap()
                        .into(),
                )
                .with_distance(5.0)
                .with_lap_number(1),
        ]);

        GarminCorrectionLap::add_mislabeled_times_to_corr_list(&mut corr_map)?;

        writeln!(stdout(), "{:?}", corr_map).unwrap();

        assert_eq!(corr_map.len(), 26);

        let key = CorrectionKey {
            datetime: convert_str_to_datetime("2010-11-20T19:55:34Z")
                .unwrap()
                .into(),
            lap_number: 0,
        };

        assert_eq!(
            corr_map.get(&key).unwrap(),
            &GarminCorrectionLap {
                id: id_0,
                start_time: convert_str_to_datetime("2010-11-20T19:55:34Z")
                    .unwrap()
                    .into(),
                lap_number: 0,
                sport: Some(SportTypes::Biking),
                distance: Some(10.0),
                duration: None,
                ..GarminCorrectionLap::default()
            }
        );
        let key = CorrectionKey {
            datetime: convert_str_to_datetime("2010-11-20T19:55:34Z")
                .unwrap()
                .into(),
            lap_number: 1,
        };
        assert_eq!(
            corr_map.get(&key).unwrap(),
            &GarminCorrectionLap {
                id: id_1,
                start_time: convert_str_to_datetime("2010-11-20T19:55:34Z")
                    .unwrap()
                    .into(),
                lap_number: 1,
                sport: Some(SportTypes::Biking),
                distance: Some(5.0),
                duration: None,
                ..GarminCorrectionLap::default()
            }
        );
        Ok(())
    }
}
