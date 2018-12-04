use chrono::Utc;
use failure::{err_msg, Error};
use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::str;

use avro_rs::{from_value, Codec, Reader, Schema, Writer};

use json::{parse, JsonValue};

use postgres::{Connection, TlsMode};

use crate::garmin_summary;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

pub fn get_corr_list_map(
    corr_list: &Vec<GarminCorrectionLap>,
) -> HashMap<(String, i32), GarminCorrectionLap> {
    corr_list
        .iter()
        .map(|corr| ((corr.start_time.clone(), corr.lap_number), corr.clone()))
        .collect()
}

pub fn corr_list_from_json(json_filename: &str) -> Result<Vec<GarminCorrectionLap>, Error> {
    let mut corr_list = Vec::new();

    let mut file = File::open(json_filename)?;

    let mut buffer = Vec::new();

    let _ = file.read_to_end(&mut buffer)?;

    let jsval = parse(&str::from_utf8(&buffer)?)?;
    match &jsval {
        JsonValue::Object(_) => {
            for (key, val) in jsval.entries() {
                match val {
                    JsonValue::Object(_) => {
                        for (lap, result) in val.entries() {
                            match result {
                                JsonValue::Number(_) => {
                                    corr_list.push(GarminCorrectionLap {
                                        id: -1,
                                        start_time: key.to_string(),
                                        lap_number: lap.parse()?,
                                        sport: None,
                                        distance: result.as_f64(),
                                        duration: None,
                                    });
                                }
                                JsonValue::Array(arr) => {
                                    corr_list.push(GarminCorrectionLap {
                                        id: -1,
                                        start_time: key.to_string(),
                                        lap_number: lap.parse()?,
                                        sport: None,
                                        distance: match arr.get(0) {
                                            Some(x) => x.as_f64(),
                                            None => None,
                                        },
                                        duration: match arr.get(1) {
                                            Some(x) => x.as_f64(),
                                            None => None,
                                        },
                                    });
                                }
                                _ => println!("something unexpected {}", result),
                            }
                        }
                    }
                    _ => println!("{}", val),
                }
            }
        }
        _ => (),
    };
    Ok(corr_list)
}

pub fn dump_corr_list_to_avro(
    corr_list: &Vec<GarminCorrectionLap>,
    output_filename: &str,
) -> Result<(), Error> {
    let garmin_file_avro_schema = GARMIN_CORRECTION_LAP_AVRO_SCHEMA;
    let schema = Schema::parse_str(&garmin_file_avro_schema)?;

    let output_file = File::create(output_filename)?;

    let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

    writer.extend_ser(corr_list)?;
    writer.flush()?;

    Ok(())
}

pub fn read_corr_list_from_avro(input_filename: &str) -> Result<Vec<GarminCorrectionLap>, Error> {
    let garmin_file_avro_schema = GARMIN_CORRECTION_LAP_AVRO_SCHEMA;
    let schema = Schema::parse_str(&garmin_file_avro_schema)?;

    let input_file = File::open(input_filename)?;

    let reader = Reader::with_schema(&schema, input_file)?;

    let mut corr_list: Vec<GarminCorrectionLap> = Vec::new();

    for record in reader {
        match from_value::<GarminCorrectionLap>(&record?) {
            Ok(v) => {
                corr_list.push(v);
                Ok(())
            }
            Err(e) => {
                println!("got here {:?}", e);
                Err(err_msg(e))
            }
        }?;
    }

    Ok(corr_list)
}

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

pub fn add_mislabeled_times_to_corr_list(
    input_corr_list: &Vec<GarminCorrectionLap>,
) -> Vec<GarminCorrectionLap> {
    let mut corr_list_map = get_corr_list_map(input_corr_list);

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
            let new_corr = match corr_list_map.get(&(time.to_string(), 0)) {
                Some(v) => GarminCorrectionLap {
                    id: v.id,
                    start_time: v.start_time.clone(),
                    lap_number: 0,
                    sport: Some(sport.to_string()),
                    distance: v.distance.clone(),
                    duration: v.duration.clone(),
                },
                None => GarminCorrectionLap {
                    id: -1,
                    start_time: time.to_string(),
                    lap_number: 0,
                    sport: Some(sport.to_string()),
                    distance: None,
                    duration: None,
                },
            };

            corr_list_map.insert((time.to_string(), 0), new_corr);
        }
    }

    corr_list_map.values().map(|v| v.clone()).collect()
}

pub fn fix_corrections() {
    let correction_file = "garmin_corrections.avro";
    let gps_dir = "/home/ddboline/.garmin_cache/run/gps_tracks";
    let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

    let corr_list = corr_list_from_json("tests/data/garmin_corrections.json")
        .expect("Failed to read corrections from json");

    let corr_list = add_mislabeled_times_to_corr_list(&corr_list);

    //dump_corr_list_to_avro(&corr_list, correction_file).expect("Failed to dump to avro");

    //let corr_list = read_corr_list_from_avro(&correction_file).expect("Failed to read avro");

    println!("{}", corr_list.len());

    let fn_unique_key_map = get_filename_start_map().expect("Failed to get filename map");

    println!(
        "{} {:?}",
        fn_unique_key_map.len(),
        fn_unique_key_map.iter().nth(0).unwrap()
    );

    let corr_map = get_corr_list_map(&corr_list);

    let gsum_list = garmin_summary::process_all_gps_files(&gps_dir, &cache_dir, &corr_map).unwrap();

    println!("{}", gsum_list.len());

    let mut new_corr_map = corr_map.clone();

    for gsum in gsum_list {
        match fn_unique_key_map.get(&gsum.filename) {
            Some((s, n)) => match corr_map.get(&(s.to_string(), *n)) {
                Some(v) => {
                    println!("{} {} {} {}", gsum.filename, gsum.begin_datetime, s, n);
                    let mut new_corr = v.clone();
                    new_corr.start_time = gsum.begin_datetime.clone();
                    new_corr_map.insert((s.to_string(), *n), new_corr);
                }
                None => (),
            },
            None => continue,
        }
    }

    let new_corr_list: Vec<GarminCorrectionLap> =
        new_corr_map.values().map(|v| v.clone()).collect();

    println!("{}", new_corr_list.len());

    dump_corr_list_to_avro(&new_corr_list, correction_file).unwrap();
}

pub fn get_filename_start_map() -> Result<HashMap<String, (String, i32)>, Error> {
    let pgstr = "postgresql://ddboline:BQGIvkKFZPejrKvX@localhost:5432/garmin_summary";
    let conn = Connection::connect(pgstr, TlsMode::None).unwrap();

    let query = "
        select filename, unique_key
        from garmin_corrections_laps a
        join garmin_summary b on a.start_time = b.begin_datetime
    ";

    let filename_start_map: HashMap<_, _> = conn.query(query, &[])?
        .iter()
        .map(|row| {
            let filename: String = row.get(0);
            let unique_key: String = row.get(1);
            let start_time: String = unique_key.split("_").nth(0).unwrap().to_string();
            let lap_number: i32 = unique_key.split("_").last().unwrap().parse().unwrap();
            (filename, (start_time, lap_number))
        })
        .collect();

    Ok(filename_start_map)
}

pub fn dump_corrections_to_db(corr_list: &Vec<GarminCorrectionLap>) {
    let pgstr = "postgresql://ddboline:BQGIvkKFZPejrKvX@localhost:5432/garmin_summary";
    let conn = Connection::connect(pgstr, TlsMode::None).unwrap();

    let query_unique_key = "SELECT unique_key FROM garmin_corrections_laps WHERE unique_key=$1";
    let query_insert = "
        INSERT INTO garmin_corrections_laps (start_time, lap_number, distance, duration, unique_key, sport)
        VALUES ($1, $2, $3, $4, $5, $6)
    ";
    let query_update = "
        UPDATE garmin_corrections_laps SET start_time=$1, lap_number=$2, distance=$3, duration=$4, sport=$6
        WHERE unique_key=$5
    ";

    let stmt_insert = conn.prepare(query_insert).unwrap();
    let stmt_update = conn.prepare(query_update).unwrap();
    for corr in corr_list {
        if corr.start_time == "DUMMY" {
            continue;
        };
        let unique_key = format!("{}_{}", corr.start_time, corr.lap_number);

        if conn.query(query_unique_key, &[&unique_key])
            .unwrap()
            .iter()
            .len() == 0
        {
            stmt_insert
                .execute(&[
                    &corr.start_time,
                    &corr.lap_number,
                    &corr.distance,
                    &corr.duration,
                    &unique_key,
                    &corr.sport,
                ])
                .unwrap();
        } else {
            stmt_update
                .execute(&[
                    &corr.start_time,
                    &corr.lap_number,
                    &corr.distance,
                    &corr.duration,
                    &unique_key,
                    &corr.sport,
                ])
                .unwrap();
        }
    }
}

pub fn read_corrections_from_db() -> Result<Vec<GarminCorrectionLap>, Error> {
    let pgstr = "postgresql://ddboline:BQGIvkKFZPejrKvX@localhost:5432/garmin_summary";
    let conn = Connection::connect(pgstr, TlsMode::None).unwrap();

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

    Ok(corr_list)
}

pub fn read_corrections_from_db_dump_to_avro() {
    let corr_list = read_corrections_from_db().unwrap();

    println!("{}", corr_list.len());

    dump_corr_list_to_avro(&corr_list, "garmin_correction.avro").unwrap();

    let corr_list = read_corr_list_from_avro("garmin_correction.avro").unwrap();

    println!("{}", corr_list.len());
}
