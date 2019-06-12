#![allow(clippy::wrong_self_convention)]

use chrono::Utc;
use failure::{err_msg, Error};
use postgres_derive::{FromSql, ToSql};
use std::collections::HashMap;
use std::fs::File;
use std::hash::BuildHasher;
use std::io::Read;
use std::str;

use avro_rs::{from_value, Codec, Reader, Schema, Writer};

use json::{parse, JsonValue};

use super::garmin_lap::GarminLap;
use super::garmin_summary::GarminSummaryList;
use super::pgpool::PgPool;
use crate::utils::garmin_util::{map_result_vec, METERS_PER_MILE};
use crate::utils::sport_types::convert_sport_name;

pub const GARMIN_CORRECTION_LAP_AVRO_SCHEMA: &str = r#"
    {
        "namespace": "garmin.avro",
        "type": "record",
        "name": "GarminCorrectionLap",
        "fields": [
            {"name": "id", "type": "int"},
            {"name": "start_time", "type": "string"},
            {"name": "lap_number", "type": "int"},
            {"name": "sport", "type": ["string", "null"]},
            {"name": "distance", "type": ["double", "null"]},
            {"name": "duration", "type": ["double", "null"]}
        ]
    }
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSql, FromSql, Default)]
pub struct GarminCorrectionLap {
    pub id: i32,
    pub start_time: String,
    pub lap_number: i32,
    pub sport: Option<String>,
    pub distance: Option<f64>,
    pub duration: Option<f64>,
}

impl GarminCorrectionLap {
    pub fn new() -> GarminCorrectionLap {
        GarminCorrectionLap {
            id: -1,
            start_time: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            lap_number: -1,
            sport: None,
            distance: None,
            duration: None,
        }
    }

    pub fn with_id(mut self, id: i32) -> GarminCorrectionLap {
        self.id = id;
        self
    }

    pub fn with_start_time(mut self, start_time: &str) -> GarminCorrectionLap {
        self.start_time = start_time.to_string();
        self
    }

    pub fn with_lap_number(mut self, lap_number: i32) -> GarminCorrectionLap {
        self.lap_number = lap_number;
        self
    }

    pub fn with_sport(mut self, sport: &str) -> GarminCorrectionLap {
        self.sport = convert_sport_name(sport);
        self
    }

    pub fn with_distance(mut self, distance: f64) -> GarminCorrectionLap {
        self.distance = Some(distance);
        self
    }

    pub fn with_duration(mut self, duration: f64) -> GarminCorrectionLap {
        self.duration = Some(duration);
        self
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct GarminCorrectionList {
    pub corr_list: Vec<GarminCorrectionLap>,
    pub pool: Option<PgPool>,
}

impl GarminCorrectionList {
    pub fn new() -> GarminCorrectionList {
        GarminCorrectionList {
            corr_list: Vec::new(),
            pool: None,
        }
    }

    pub fn with_vec(mut self, corr_list: Vec<GarminCorrectionLap>) -> GarminCorrectionList {
        self.corr_list = corr_list;
        self
    }

    pub fn with_pool(mut self, pool: &PgPool) -> GarminCorrectionList {
        self.pool = Some(pool.clone());
        self
    }
}

impl GarminCorrectionListTrait for GarminCorrectionList {
    fn from_pool(pool: &PgPool) -> GarminCorrectionList {
        GarminCorrectionList {
            pool: Some(pool.clone()),
            ..Default::default()
        }
    }

    fn get_pool(&self) -> Result<&PgPool, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| err_msg("No Database Connection"))
    }

    fn get_corr_list(&self) -> &Vec<GarminCorrectionLap> {
        &self.corr_list
    }

    fn from_vec(corr_list: Vec<GarminCorrectionLap>) -> GarminCorrectionList {
        GarminCorrectionList {
            corr_list,
            pool: None,
        }
    }
}

pub trait GarminCorrectionListTrait
where
    Self: Sized,
{
    fn from_pool(pool: &PgPool) -> Self;

    fn from_vec(corr_list: Vec<GarminCorrectionLap>) -> Self;

    fn get_pool(&self) -> Result<&PgPool, Error>;

    fn get_corr_list(&self) -> &Vec<GarminCorrectionLap>;

    fn get_corr_list_map(&self) -> HashMap<(String, i32), GarminCorrectionLap> {
        self.get_corr_list()
            .iter()
            .map(|corr| ((corr.start_time.clone(), corr.lap_number), corr.clone()))
            .collect()
    }

    fn corr_list_from_buffer(buffer: &[u8]) -> Result<Self, Error> {
        let jsval = parse(&str::from_utf8(&buffer)?)?;

        let corr_list = match &jsval {
            JsonValue::Object(_) => jsval
                .entries()
                .flat_map(|(key, val)| match val {
                    JsonValue::Object(_) => val
                        .entries()
                        .map(|(lap, result)| match result {
                            JsonValue::Number(_) => {
                                let corr = GarminCorrectionLap::new()
                                    .with_start_time(&key)
                                    .with_lap_number(lap.parse()?);
                                Ok(match result.as_f64() {
                                    Some(r) => corr.with_distance(r),
                                    None => corr,
                                })
                            }
                            JsonValue::Array(arr) => {
                                let corr = GarminCorrectionLap::new()
                                    .with_start_time(&key)
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
                            _ => Err(err_msg(format!("something unexpected {}", result))),
                        })
                        .collect(),
                    _ => Vec::new(),
                })
                .filter_map(|x| match x {
                    Ok(s) => Some(s),
                    Err(e) => {
                        debug!("Error {}", e);
                        None
                    }
                })
                .collect(),
            _ => Vec::new(),
        };

        Ok(Self::from_vec(corr_list))
    }

    fn corr_list_from_json(json_filename: &str) -> Result<Self, Error> {
        let mut file = File::open(json_filename)?;

        let mut buffer = Vec::new();

        match file.read_to_end(&mut buffer)? {
            0 => Err(err_msg(format!("Zero bytes read from {}", json_filename))),
            _ => Self::corr_list_from_buffer(&buffer),
        }
    }

    fn dump_corr_list_to_avro(&self, output_filename: &str) -> Result<(), Error> {
        let garmin_file_avro_schema = GARMIN_CORRECTION_LAP_AVRO_SCHEMA;
        let schema = Schema::parse_str(&garmin_file_avro_schema)?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer.extend_ser(self.get_corr_list())?;
        writer.flush()?;

        Ok(())
    }

    fn read_corr_list_from_avro(input_filename: &str) -> Result<Self, Error> {
        let garmin_file_avro_schema = GARMIN_CORRECTION_LAP_AVRO_SCHEMA;
        let schema = Schema::parse_str(&garmin_file_avro_schema)?;

        let input_file = File::open(input_filename)?;

        let reader = Reader::with_schema(&schema, input_file)?;

        let corr_list: Vec<Result<_, Error>> = reader
            .map(|record| match from_value::<GarminCorrectionLap>(&record?) {
                Ok(v) => Ok(v),
                Err(e) => Err(err_msg(e)),
            })
            .collect();

        let corr_list = map_result_vec(corr_list)?;

        Ok(Self::from_vec(corr_list))
    }

    fn add_mislabeled_times_to_corr_list(&self) -> Self {
        let mut corr_list_map = self.get_corr_list_map();

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
            for time in times_list {
                let lap_list: Vec<_> = corr_list_map
                    .keys()
                    .filter_map(|(t, n)| if t == time { Some(*n) } else { None })
                    .collect();

                let lap_list = if !lap_list.is_empty() {
                    lap_list
                } else {
                    vec![0]
                };

                for lap_number in lap_list {
                    let new_corr = match corr_list_map.get(&(time.to_string(), lap_number)) {
                        Some(v) => v.clone().with_sport(sport),
                        None => GarminCorrectionLap::new()
                            .with_start_time(time)
                            .with_lap_number(lap_number)
                            .with_sport(sport),
                    };

                    corr_list_map.insert((time.to_string(), lap_number), new_corr);
                }
            }
        }

        Self::from_vec(corr_list_map.values().cloned().collect())
    }

    fn fix_corrections(&self) -> Result<(), Error> {
        let correction_file = "garmin_corrections.avro";
        let gps_dir = "/home/ddboline/.garmin_cache/run/gps_tracks";
        let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

        let corr_list = Self::corr_list_from_json("tests/data/garmin_corrections.json")?;

        let corr_list = corr_list.add_mislabeled_times_to_corr_list();

        //dump_corr_list_to_avro(&corr_list, correction_file)?;

        //let corr_list = read_corr_list_from_avro(&correction_file)?;

        println!("{}", corr_list.get_corr_list().len());

        let fn_unique_key_map = self.get_filename_start_map()?;

        println!(
            "{} {:?}",
            fn_unique_key_map.len(),
            fn_unique_key_map.iter().nth(0)
        );

        let corr_map = corr_list.get_corr_list_map();

        let gsum_list = GarminSummaryList::process_all_gps_files(&gps_dir, &cache_dir, &corr_map)?;

        println!("{}", gsum_list.summary_list.len());

        let mut new_corr_map = corr_map.clone();

        for gsum in gsum_list.summary_list {
            match fn_unique_key_map.get(&gsum.filename) {
                Some((s, n)) => {
                    if let Some(v) = corr_map.get(&(s.to_string(), *n)) {
                        println!("{} {} {} {}", gsum.filename, gsum.begin_datetime, s, n);
                        let mut new_corr = v.clone();
                        new_corr.start_time = gsum.begin_datetime.clone();
                        new_corr_map.insert((s.to_string(), *n), new_corr);
                    }
                }
                None => continue,
            }
        }

        let new_corr_list = Self::from_vec(new_corr_map.values().cloned().collect());

        println!("{}", new_corr_list.get_corr_list().len());

        new_corr_list.dump_corr_list_to_avro(correction_file)?;
        Ok(())
    }

    fn get_filename_start_map(&self) -> Result<HashMap<String, (String, i32)>, Error> {
        let query = "
            select filename, unique_key
            from garmin_corrections_laps a
            join garmin_summary b on a.start_time = b.begin_datetime
        ";
        let conn = self.get_pool()?.get()?;
        let filename_start_map: HashMap<_, _> = conn
            .query(query, &[])?
            .iter()
            .map(|row| {
                let filename: String = row.get(0);
                let unique_key: String = row.get(1);
                let start_time: String = unique_key
                    .split('_')
                    .nth(0)
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| "".to_string());
                let lap_number: i32 = unique_key
                    .split('_')
                    .last()
                    .map(|x| x.parse().unwrap_or(0))
                    .unwrap_or(0);
                (filename, (start_time, lap_number))
            })
            .collect();

        Ok(filename_start_map)
    }

    fn dump_corrections_to_db(&self) -> Result<(), Error> {
        let query_unique_key = "SELECT unique_key FROM garmin_corrections_laps WHERE unique_key=$1";
        let query_insert = "
            INSERT INTO garmin_corrections_laps (start_time, lap_number, distance, duration, unique_key, sport)
            VALUES ($1, $2, $3, $4, $5, $6)
        ";
        let query_update = "
            UPDATE garmin_corrections_laps SET start_time=$1, lap_number=$2, distance=$3, duration=$4, sport=$6
            WHERE unique_key=$5
        ";
        let conn = self.get_pool()?.get()?;
        let stmt_insert = conn.prepare(query_insert)?;
        let stmt_update = conn.prepare(query_update)?;
        for corr in self.get_corr_list() {
            if corr.start_time == "DUMMY" {
                continue;
            };
            let unique_key = format!("{}_{}", corr.start_time, corr.lap_number);

            if conn.query(query_unique_key, &[&unique_key])?.iter().len() == 0 {
                stmt_insert.execute(&[
                    &corr.start_time,
                    &corr.lap_number,
                    &corr.distance,
                    &corr.duration,
                    &unique_key,
                    &corr.sport,
                ])?;
            } else {
                stmt_update.execute(&[
                    &corr.start_time,
                    &corr.lap_number,
                    &corr.distance,
                    &corr.duration,
                    &unique_key,
                    &corr.sport,
                ])?;
            }
        }
        Ok(())
    }

    fn read_corrections_from_db(&self) -> Result<Self, Error> {
        let conn = self.get_pool()?.get()?;
        let corr_list: Vec<GarminCorrectionLap> = conn.query(
            "select id, start_time, lap_number, sport, distance, duration from garmin_corrections_laps",
            &[],
        )?
            .iter()
            .map(|row| GarminCorrectionLap {
                id: row.get(0),
                start_time: row.get(1),
                lap_number: row.get(2),
                sport: row.get(3),
                distance: row.get(4),
                duration: row.get(5),
            })
            .collect();

        Ok(Self::from_vec(corr_list))
    }

    fn read_corrections_from_db_dump_to_avro(&self) -> Result<(), Error> {
        let corr_list = self.read_corrections_from_db()?;

        println!("{}", corr_list.get_corr_list().len());

        corr_list.dump_corr_list_to_avro("garmin_correction.avro")?;

        let corr_list = Self::read_corr_list_from_avro("garmin_correction.avro")?;

        println!("{}", corr_list.get_corr_list().len());
        Ok(())
    }
}

pub fn apply_lap_corrections<S: BuildHasher + Sync>(
    lap_list: &[GarminLap],
    sport: &Option<String>,
    corr_map: &HashMap<(String, i32), GarminCorrectionLap, S>,
) -> (Vec<GarminLap>, Option<String>) {
    let mut new_sport = sport.clone();
    match lap_list.get(0) {
        Some(l) => {
            let lap_start = &l.lap_start.clone();
            for lap in lap_list {
                debug!("lap {} dis {}", lap.lap_number, lap.lap_distance);
            }
            let new_lap_list: Vec<_> = lap_list
                .iter()
                .map(|lap| {
                    let lap_number = lap.lap_number;
                    match &corr_map.get(&(lap_start.to_string(), lap_number)) {
                        Some(corr) => {
                            let mut new_lap = lap.clone();
                            new_sport = match &corr.sport {
                                Some(s) => {
                                    debug!("change sport {} {:?} {}", lap_start, lap.lap_type, s);
                                    Some(s.clone())
                                }
                                None => sport.clone(),
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
