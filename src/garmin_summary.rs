extern crate flate2;
extern crate rand;
extern crate rayon;

use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use postgres_derive::{FromSql, ToSql};

use std::path::Path;

use std::fs::File;

use failure::Error;

use rayon::prelude::*;

use std::collections::HashMap;
use std::fmt;

use postgres::Connection;

use crate::garmin_correction_lap::{
    get_corr_list_map, read_corrections_from_db, GarminCorrectionLap,
};
use crate::garmin_file::GarminFile;
use crate::parsers::garmin_parse::GarminParse;
use crate::utils::garmin_util::{generate_random_string, get_md5sum, map_result_vec};

#[derive(Debug, Clone, Serialize, Deserialize, ToSql, FromSql)]
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
    let md5sum = get_md5sum(&filename)?;
    let gfile = GarminParse::new(&filename, &corr_map);
    match gfile.laps.get(0) {
        Some(_) => (),
        None => println!("{} has no laps?", gfile.filename),
    };
    gfile.dump_avro(&cache_file)?;
    Ok(GarminSummary::new(&gfile, &md5sum))
}

pub fn get_file_list(path: &Path) -> Vec<String> {
    match path.read_dir() {
        Ok(it) => it
            .filter_map(|dir_line| match dir_line {
                Ok(entry) => {
                    let input_file = entry.path().to_str().unwrap().to_string();
                    Some(input_file)
                }
                Err(_) => None,
            })
            .collect(),
        Err(err) => {
            println!("{}", err);
            Vec::new()
        }
    }
}

pub fn process_all_gps_files(
    gps_dir: &str,
    cache_dir: &str,
    corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
) -> Result<Vec<GarminSummary>, Error> {
    let path = Path::new(gps_dir);

    let file_list: Vec<String> = get_file_list(&path);

    let gsum_result_list: Vec<Result<GarminSummary, Error>> = file_list
        .par_iter()
        .map(|input_file| {
            let cache_file = format!(
                "{}/{}.avro",
                cache_dir,
                input_file.split("/").last().unwrap()
            );
            let md5sum = get_md5sum(&input_file)?;
            let gfile = GarminParse::new(&input_file, &corr_map);
            match gfile.laps.get(0) {
                Some(_) => (),
                None => println!("{} {} has no laps?", &input_file, &gfile.filename),
            };
            gfile.dump_avro(&cache_file)?;
            Ok(GarminSummary::new(&gfile, &md5sum))
        })
        .collect();

    Ok(map_result_vec(gsum_result_list)?)
}

pub fn create_summary_list(conn: &Connection) -> Result<Vec<GarminSummary>, Error> {
    let gps_dir = "/home/ddboline/.garmin_cache/run/gps_tracks";
    let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

    let corr_list = read_corrections_from_db(&conn)?;

    println!("{}", corr_list.len());

    let corr_map = get_corr_list_map(&corr_list);

    process_all_gps_files(&gps_dir, &cache_dir, &corr_map)
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

pub fn read_summary_from_avro(input_filename: &str) -> Result<Vec<GarminSummary>, Error> {
    let garmin_summary_avro_schema = GARMIN_SUMMARY_AVRO_SCHEMA;
    let schema = Schema::parse_str(&garmin_summary_avro_schema)?;

    let input_file = File::open(input_filename)?;

    let reader = Reader::with_schema(&schema, input_file)?;

    let mut gsum_list = Vec::new();

    for record in reader {
        match record {
            Ok(r) => match from_value::<GarminSummary>(&r) {
                Ok(v) => {
                    debug!("{:?}", v);
                    gsum_list.push(v);
                }
                Err(e) => {
                    debug!("got here 0 {:?}", e);
                    continue;
                }
            },
            Err(e) => {
                debug!("got here 1 {:?}", e);
                continue;
            }
        };
    }

    Ok(gsum_list)
}

pub fn read_summary_from_postgres(conn: &Connection) -> Result<Vec<GarminSummary>, Error> {
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

    let gsum_list = conn
        .query(&query, &[])?
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
    conn: &Connection,
    pattern: &str,
) -> Result<Vec<GarminSummary>, Error> {
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

    let gsum_list = conn
        .query(&query, &[])?
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

pub fn write_summary_to_avro_files(
    gsum_list: &Vec<GarminSummary>,
    summary_cache_dir: &str,
) -> Result<(), Error> {
    let results = gsum_list
        .iter()
        .map(|gsum| {
            let summary_avro_fname =
                format!("{}/{}.summary.avro", &summary_cache_dir, &gsum.filename);
            let single_summary = vec![gsum.clone()];
            dump_summary_to_avro(&single_summary, &summary_avro_fname)
        })
        .collect();

    map_result_vec(results)?;
    Ok(())
}

pub fn read_summary_from_avro_files(summary_cache_dir: &str) -> Result<Vec<GarminSummary>, Error> {
    let path = Path::new(summary_cache_dir);

    let file_list: Vec<String> = get_file_list(&path);

    let gsum_result_list: Vec<_> = file_list
        .par_iter()
        .map(|f| read_summary_from_avro(f))
        .collect();

    Ok(map_result_vec(gsum_result_list)?
        .into_iter()
        .flatten()
        .collect())
}

pub fn get_list_of_files_from_db(conn: &Connection) -> Result<Vec<String>, Error> {
    let filename_query = "SELECT filename FROM garmin_summary";

    Ok(conn
        .query(filename_query, &[])?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

pub fn write_summary_to_postgres(
    conn: &Connection,
    gsum_list: &Vec<GarminSummary>,
) -> Result<(), Error> {
    let rand_str = generate_random_string(8);

    let temp_table_name = format!("garmin_summary_{}", rand_str);

    let create_table_query = format!(
        "CREATE TABLE {} (
            filename text NOT NULL PRIMARY KEY,
            begin_datetime text,
            sport varchar(12),
            total_calories integer,
            total_distance double precision,
            total_duration double precision,
            total_hr_dur double precision,
            total_hr_dis double precision,
            number_of_items integer,
            md5sum varchar(32)
        );",
        temp_table_name
    );

    conn.execute(&create_table_query, &[])?;

    let insert_query = format!("
        INSERT INTO {} (filename, begin_datetime, sport, total_calories, total_distance, total_duration, total_hr_dur, total_hr_dis, md5sum, number_of_items)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1)
    ", temp_table_name);

    let stmt_insert = conn.prepare(&insert_query)?;

    for gsum in gsum_list {
        stmt_insert.execute(&[
            &gsum.filename,
            &gsum.begin_datetime,
            &gsum.sport,
            &gsum.total_calories,
            &gsum.total_distance,
            &gsum.total_duration,
            &gsum.total_hr_dur,
            &gsum.total_hr_dis,
            &gsum.md5sum,
        ])?;
    }

    let insert_query = format!("
        INSERT INTO garmin_summary (filename, begin_datetime, sport, total_calories, total_distance, total_duration, total_hr_dur, total_hr_dis, md5sum, number_of_items)
        SELECT b.filename, b.begin_datetime, b.sport, b.total_calories, b.total_distance, b.total_duration, b.total_hr_dur, b.total_hr_dis, b.md5sum, b.number_of_items
        FROM {} b
        WHERE b.filename not in (select filename from garmin_summary)
    ", temp_table_name);

    let update_query = format!("
        UPDATE garmin_summary a
        SET (begin_datetime,sport,total_calories,total_distance,total_duration,total_hr_dur,total_hr_dis,md5sum,number_of_items) = 
            (b.begin_datetime,b.sport,b.total_calories,b.total_distance,b.total_duration,b.total_hr_dur,b.total_hr_dis,b.md5sum,b.number_of_items)
        FROM {} b
        WHERE a.filename = b.filename
    ", temp_table_name);

    let drop_table_query = format!("DROP TABLE {}", temp_table_name);

    conn.execute(&insert_query, &[])?;
    conn.execute(&update_query, &[])?;
    conn.execute(&drop_table_query, &[])?;

    Ok(())
}

pub fn dump_summary_from_postgres_to_avro(conn: &Connection) -> Result<(), Error> {
    let gsum_list = read_summary_from_postgres(&conn)?;

    println!("{}", gsum_list.len());

    dump_summary_to_avro(&gsum_list, "garmin_summary.avro")?;

    let gsum_list = read_summary_from_avro("garmin_summary.avro")?;

    println!("{}", gsum_list.len());
    Ok(())
}
