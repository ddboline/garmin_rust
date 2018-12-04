extern crate flate2;
extern crate rayon;

use avro_rs::{from_value, Codec, Reader, Schema, Writer};

use std::path::Path;

use std::fs::File;

use failure::Error;

use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt;

use postgres::{Connection, TlsMode};

use crate::garmin_correction_lap::{
    get_corr_list_map, read_corrections_from_db, GarminCorrectionLap,
};
use crate::garmin_file::GarminFile;
use crate::garmin_parse::GarminParse;
use crate::garmin_util::get_md5sum;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminSummary {
    pub filename: String,
    pub begin_datetime: String,
    pub sport: String,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub number_of_items: i32,
    pub md5sum: String,
}

impl GarminSummary {
    pub fn new(gfile: &GarminFile, md5sum: &str) -> GarminSummary {
        GarminSummary {
            filename: gfile.filename.clone(),
            begin_datetime: gfile.begin_datetime.clone(),
            sport: gfile.sport.clone().unwrap_or("".to_string()),
            total_calories: gfile.total_calories,
            total_distance: gfile.total_distance,
            total_duration: gfile.total_duration,
            total_hr_dur: gfile.total_hr_dur,
            total_hr_dis: gfile.total_hr_dis,
            number_of_items: 1,
            md5sum: md5sum.to_string(),
        }
    }
}

impl fmt::Display for GarminSummary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let keys = vec![
            "filename",
            "begin_datetime",
            "sport",
            "total_calories",
            "total_distance",
            "total_duration",
            "total_hr_dur",
            "total_hr_dis",
            "number_of_items",
            "md5sum",
        ];
        let vals = vec![
            self.filename.to_string(),
            self.begin_datetime.to_string(),
            self.sport.to_string(),
            self.total_calories.to_string(),
            self.total_distance.to_string(),
            self.total_duration.to_string(),
            self.total_hr_dur.to_string(),
            self.total_hr_dis.to_string(),
            self.number_of_items.to_string(),
            self.md5sum.to_string(),
        ];
        write!(
            f,
            "GarminSummaryTable<{}>",
            keys.iter()
                .zip(vals.iter())
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

pub const GARMIN_SUMMARY_AVRO_SCHEMA: &str = r#"
    {
        "namespace": "garmin.avro",
        "type": "record",
        "name": "GarminSummary",
        "fields": [
            {"name": "filename", "type": "string"},
            {"name": "begin_datetime", "type": "string"},
            {"name": "sport", "type": "string"},
            {"name": "total_calories", "type": "int"},
            {"name": "total_distance", "type": "double"},
            {"name": "total_duration", "type": "double"},
            {"name": "total_hr_dur", "type": "double"},
            {"name": "total_hr_dis", "type": "double"},
            {"name": "number_of_items", "type": "int"},
            {"name": "md5sum", "type": "string"}
        ]
    }
"#;

pub fn process_single_gps_file(
    filename: &str,
    cache_dir: &str,
    corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
) -> Result<GarminSummary, Error> {
    let cache_file = format!("{}/{}.avro", cache_dir, filename.split("/").last().unwrap());
    let md5sum = get_md5sum(&filename);
    let gfile = GarminParse::new(&filename, &corr_map);
    match gfile.laps.get(0) {
        Some(_) => (),
        None => println!("{} has no laps?", gfile.filename),
    };
    gfile.dump_avro(&cache_file)?;
    Ok(GarminSummary::new(&gfile, &md5sum))
}

pub fn process_all_gps_files(
    gps_dir: &str,
    cache_dir: &str,
    corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
) -> Result<Vec<GarminSummary>, Error> {
    let path = Path::new(gps_dir);

    let file_list: Vec<String> = match path.read_dir() {
        Ok(it) => it.filter_map(|dir_line| match dir_line {
            Ok(entry) => {
                let input_file = entry.path().to_str().unwrap().to_string();
                Some(input_file)
            }
            Err(_) => None,
        }).collect(),
        Err(err) => {
            println!("{}", err);
            Vec::new()
        }
    };

    let gsum_list: Vec<GarminSummary> = file_list
        .par_iter()
        .map(|input_file| {
            let cache_file = format!(
                "{}/{}.avro",
                cache_dir,
                input_file.split("/").last().unwrap()
            );
            let md5sum = get_md5sum(&input_file);
            let gfile = GarminParse::new(&input_file, &corr_map);
            match gfile.laps.get(0) {
                Some(_) => (),
                None => println!("{} {} has no laps?", &input_file, &gfile.filename),
            };
            gfile.dump_avro(&cache_file).unwrap();
            GarminSummary::new(&gfile, &md5sum)
        })
        .collect();
    Ok(gsum_list)
}

pub fn create_summary_list(pg_url: &str) -> Vec<GarminSummary> {
    let gps_dir = "/home/ddboline/.garmin_cache/run/gps_tracks";
    let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

    let corr_list = read_corrections_from_db(pg_url).unwrap();

    println!("{}", corr_list.len());

    let corr_map = get_corr_list_map(&corr_list);

    process_all_gps_files(&gps_dir, &cache_dir, &corr_map).unwrap()
}

pub fn dump_summary_to_avro(
    gsum_list: &Vec<GarminSummary>,
    output_filename: &str,
) -> Result<(), Error> {
    let garmin_summary_avro_schema = GARMIN_SUMMARY_AVRO_SCHEMA;
    let schema = Schema::parse_str(&garmin_summary_avro_schema)?;

    let output_file = File::create(output_filename)?;

    let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

    writer.extend_ser(gsum_list)?;
    writer.flush()?;

    Ok(())
}

pub fn read_summary_from_avro(input_filename: &str) -> Vec<GarminSummary> {
    let garmin_summary_avro_schema = GARMIN_SUMMARY_AVRO_SCHEMA;
    let schema = Schema::parse_str(&garmin_summary_avro_schema).expect("Failed to parse schema");

    let input_file = File::open(input_filename).expect("Failed to open file");

    let reader = Reader::with_schema(&schema, input_file).expect("Failed to intialize reader");

    let mut gsum_list = Vec::new();

    for record in reader {
        match record {
            Ok(r) => match from_value::<GarminSummary>(&r) {
                Ok(v) => {
                    println!("{:?}", v);
                    gsum_list.push(v);
                }
                Err(e) => {
                    println!("got here 0 {:?}", e);
                    continue;
                }
            },
            Err(e) => {
                println!("got here 1 {:?}", e);
                continue;
            }
        };
    }

    gsum_list
}

pub fn read_summary_from_postgres(pg_url: &str) -> Result<Vec<GarminSummary>, Error> {
    let conn = Connection::connect(pg_url, TlsMode::None).unwrap();

    let query = "
        SELECT filename,
               begin_datetime,
               sport,
               total_calories,
               total_distance,
               total_duration,
               total_hr_dur,
               total_hr_dis,
               number_of_items,
               md5sum
        FROM garmin_summary
    ";

    let gsum_list = conn.query(&query, &[])?
        .iter()
        .map(|row| GarminSummary {
            filename: row.get(0),
            begin_datetime: row.get(1),
            sport: row.get(2),
            total_calories: row.get(3),
            total_distance: row.get(4),
            total_duration: row.get(5),
            total_hr_dur: row.get(6),
            total_hr_dis: row.get(7),
            number_of_items: row.get(8),
            md5sum: row.get(9),
        })
        .collect();
    Ok(gsum_list)
}

pub fn read_summary_from_postgres_pattern(
    pg_url: &str,
    pattern: &str,
) -> Result<Vec<GarminSummary>, Error> {
    let conn = Connection::connect(pg_url, TlsMode::None).unwrap();

    let query = format!(
        "
        SELECT filename,
               begin_datetime,
               sport,
               total_calories,
               total_distance,
               total_duration,
               total_hr_dur,
               total_hr_dis,
               number_of_items,
               md5sum
        FROM garmin_summary
        WHERE filename like '%{}%'
    ",
        pattern
    );

    let gsum_list = conn.query(&query, &[])?
        .iter()
        .map(|row| GarminSummary {
            filename: row.get(0),
            begin_datetime: row.get(1),
            sport: row.get(2),
            total_calories: row.get(3),
            total_distance: row.get(4),
            total_duration: row.get(5),
            total_hr_dur: row.get(6),
            total_hr_dis: row.get(7),
            number_of_items: row.get(8),
            md5sum: row.get(9),
        })
        .collect();
    Ok(gsum_list)
}

pub fn write_summary_to_postgres(pg_url: &str, gsum_list: &Vec<GarminSummary>) {
    let conn = Connection::connect(pg_url, TlsMode::None).unwrap();

    let filename_query = "SELECT filename FROM garmin_summary WHERE filename=$1";

    let insert_query = "
        INSERT INTO garmin_summary (filename, begin_datetime, sport, total_calories, total_distance, total_duration, total_hr_dur, total_hr_dis, md5sum, number_of_items)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1)
    ";

    let update_query = "
        UPDATE garmin_summary SET (begin_datetime,sport,total_calories,total_distance,total_duration,total_hr_dur,total_hr_dis,md5sum,number_of_items) = ($2,$3,$4,$5,$6,$7,$8,$9,1)
        WHERE filename=$1
    ";

    let stmt_insert = conn.prepare(insert_query).unwrap();
    let stmt_update = conn.prepare(update_query).unwrap();

    for gsum in gsum_list {
        let existing = conn.query(filename_query, &[&gsum.filename])
            .unwrap()
            .iter()
            .len();

        if existing == 0 {
            stmt_insert
                .execute(&[
                    &gsum.filename,
                    &gsum.begin_datetime,
                    &gsum.sport,
                    &gsum.total_calories,
                    &gsum.total_distance,
                    &gsum.total_duration,
                    &gsum.total_hr_dur,
                    &gsum.total_hr_dis,
                    &gsum.md5sum,
                ])
                .unwrap();
        } else {
            stmt_update
                .execute(&[
                    &gsum.filename,
                    &gsum.begin_datetime,
                    &gsum.sport,
                    &gsum.total_calories,
                    &gsum.total_distance,
                    &gsum.total_duration,
                    &gsum.total_hr_dur,
                    &gsum.total_hr_dis,
                    &gsum.md5sum,
                ])
                .unwrap();
        };
    }
}

pub fn dump_summary_from_postgres_to_avro(pg_url: &str) {
    let gsum_list = read_summary_from_postgres(&pg_url).unwrap();

    println!("{}", gsum_list.len());

    dump_summary_to_avro(&gsum_list, "garmin_summary.avro").unwrap();

    let gsum_list = read_summary_from_avro("garmin_summary.avro");

    println!("{}", gsum_list.len());
}
