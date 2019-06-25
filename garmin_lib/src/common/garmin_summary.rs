use avro_rs::{from_value, Codec, Reader, Schema, Writer};
use postgres_derive::{FromSql, ToSql};
use tempdir::TempDir;

use std::path::Path;

use std::fs::File;

use failure::{err_msg, Error};

use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};

use std::collections::HashMap;
use std::fmt;

use super::garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionList};
use super::garmin_file::GarminFile;
use super::garmin_sync::GarminSync;
use super::pgpool::PgPool;
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::utils::garmin_util::{generate_random_string, get_file_list, get_md5sum, map_result};
use crate::utils::row_index_trait::RowIndexTrait;

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

#[derive(Debug, Clone, Serialize, Deserialize, ToSql, FromSql, Default)]
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
            sport: gfile.sport.clone().unwrap_or_else(|| "".to_string()),
            total_calories: gfile.total_calories,
            total_distance: gfile.total_distance,
            total_duration: gfile.total_duration,
            total_hr_dur: gfile.total_hr_dur,
            total_hr_dis: gfile.total_hr_dis,
            number_of_items: 1,
            md5sum: md5sum.to_string(),
        }
    }

    pub fn process_single_gps_file(
        filename: &str,
        cache_dir: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummary, Error> {
        let cache_file = format!(
            "{}/{}.avro",
            cache_dir,
            filename
                .split('/')
                .last()
                .ok_or_else(|| err_msg(format!("Failed to split filename {}", filename)))?
        );

        println!("Get md5sum {}", filename);
        let md5sum = get_md5sum(&filename)?;

        println!("Found md5sum {}, try parsing", md5sum);
        let gfile = GarminParse::new().with_file(&filename, &corr_map)?;

        match gfile.laps.get(0) {
            Some(l) if l.lap_start.is_empty() => {
                return Err(err_msg(format!("{} has empty lap start?", &gfile.filename)));
            }
            Some(_) => (),
            None => return Err(err_msg(format!("{} has no laps?", gfile.filename))),
        };
        gfile.dump_avro(&cache_file)?;
        Ok(GarminSummary::new(&gfile, &md5sum))
    }

    pub fn process_and_upload_single_gps_file(
        filename: &str,
        gps_bucket: &str,
        cache_bucket: &str,
        summary_bucket: &str,
    ) -> Result<(), Error> {
        let tempdir = TempDir::new("garmin_cache")?;

        let temp_path = tempdir
            .path()
            .to_str()
            .ok_or_else(|| err_msg("Path is invalid unicode somehow"))?;

        let corr_file = format!("{}/{}", temp_path, "garmin_correction.avro");

        let gsync = GarminSync::new();
        gsync.download_file(&corr_file, &cache_bucket, "garmin_correction.avro")?;

        debug!("Try downloading {}", corr_file);
        let corr_list = GarminCorrectionList::read_corr_list_from_avro(&corr_file)?;
        debug!("Success {}", corr_list.corr_list.len());
        let corr_map = corr_list.get_corr_list_map();

        let local_file = format!("{}/{}", temp_path, filename);

        debug!("Download file {}", local_file);
        let md5sum = gsync
            .download_file(&local_file, &gps_bucket, &filename)?
            .trim_matches('"')
            .to_string();

        debug!("Try processing file {} {}", local_file, temp_path);

        let cache_file = format!(
            "{}/{}.avro",
            temp_path,
            filename
                .split('/')
                .last()
                .ok_or_else(|| err_msg(format!("Failed to split filename {}", filename)))?
        );

        println!("Found md5sum {}, try parsing", md5sum);
        let gfile = GarminParse::new().with_file(&filename, &corr_map)?;

        match gfile.laps.get(0) {
            Some(_) => (),
            None => println!("{} has no laps?", gfile.filename),
        };
        gfile.dump_avro(&cache_file)?;
        let gsum = GarminSummary::new(&gfile, &md5sum);

        let gsum_list = GarminSummaryList::from_vec(vec![gsum]);

        gsum_list.write_summary_to_avro_files(&temp_path)?;

        let local_file = format!("{}/{}.avro", temp_path, filename);
        let s3_key = format!("{}.avro", filename);

        gsync.upload_file(&local_file, &cache_bucket, &s3_key)?;

        let local_file = format!("{}/{}.summary.avro", temp_path, filename);
        let s3_key = format!("{}.summary.avro", filename);

        gsync.upload_file(&local_file, &summary_bucket, &s3_key)?;
        Ok(())
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

#[derive(Default)]
pub struct GarminSummaryList {
    pub summary_list: Vec<GarminSummary>,
    pub pool: Option<PgPool>,
}

impl GarminSummaryList {
    pub fn new() -> GarminSummaryList {
        GarminSummaryList {
            summary_list: Vec::new(),
            pool: None,
        }
    }

    pub fn from_vec(summary_list: Vec<GarminSummary>) -> GarminSummaryList {
        GarminSummaryList {
            summary_list,
            pool: None,
        }
    }

    pub fn with_pool(mut self, pool: &PgPool) -> GarminSummaryList {
        self.pool = Some(pool.clone());
        self
    }

    pub fn from_pool(pool: &PgPool) -> GarminSummaryList {
        GarminSummaryList {
            summary_list: Vec::new(),
            pool: Some(pool.clone()),
        }
    }

    pub fn get_pool(&self) -> Result<PgPool, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| err_msg("No Database Connection"))
            .map(|x| x.clone())
    }

    pub fn process_all_gps_files(
        gps_dir: &str,
        cache_dir: &str,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let path = Path::new(gps_dir);

        let gsum_result_list: Vec<Result<_, Error>> = get_file_list(&path)
            .into_par_iter()
            .map(|input_file| {
                println!("Process {}", &input_file);
                let cache_file = format!(
                    "{}/{}.avro",
                    cache_dir,
                    input_file.split('/').last().ok_or_else(|| err_msg(format!(
                        "Failed to split input_file {}",
                        input_file
                    )))?
                );
                let md5sum = get_md5sum(&input_file)?;
                let gfile = GarminParse::new().with_file(&input_file, &corr_map)?;
                match gfile.laps.get(0) {
                    Some(l) if l.lap_start.is_empty() => {
                        return Err(err_msg(format!(
                            "{} {} has empty lap start?",
                            &input_file, &gfile.filename
                        )));
                    }
                    Some(_) => (),
                    None => {
                        return Err(err_msg(format!(
                            "{} {} has no laps?",
                            &input_file, &gfile.filename
                        )));
                    }
                };
                gfile.dump_avro(&cache_file)?;
                Ok(GarminSummary::new(&gfile, &md5sum))
            })
            .collect();

        Ok(GarminSummaryList::from_vec(map_result(gsum_result_list)?))
    }

    pub fn create_summary_list(&self) -> Result<GarminSummaryList, Error> {
        let gps_dir = "/home/ddboline/.garmin_cache/run/gps_tracks";
        let cache_dir = "/home/ddboline/.garmin_cache/run/cache";

        let corr_list =
            GarminCorrectionList::from_pool(&self.get_pool()?).read_corrections_from_db()?;

        println!("{}", corr_list.corr_list.len());

        let corr_map = corr_list.get_corr_list_map();

        GarminSummaryList::process_all_gps_files(&gps_dir, &cache_dir, &corr_map)
    }

    pub fn read_summary_from_avro(input_filename: &str) -> Result<GarminSummaryList, Error> {
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

        Ok(GarminSummaryList::from_vec(gsum_list))
    }

    pub fn read_summary_from_postgres(&self, pattern: &str) -> Result<GarminSummaryList, Error> {
        let where_str = if !pattern.is_empty() {
            format!("WHERE filename like '%{}%'", pattern)
        } else {
            "".to_string()
        };

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
            {}",
            where_str
        );
        let pool = self.get_pool()?;
        let conn = pool.get()?;

        let gsum_list: Vec<_> = conn
            .query(&query, &[])?
            .iter()
            .map(|row| {
                Ok(GarminSummary {
                    filename: row.get_idx(0)?,
                    begin_datetime: row.get_idx(1)?,
                    sport: row.get_idx(2)?,
                    total_calories: row.get_idx(3)?,
                    total_distance: row.get_idx(4)?,
                    total_duration: row.get_idx(5)?,
                    total_hr_dur: row.get_idx(6)?,
                    total_hr_dis: row.get_idx(7)?,
                    number_of_items: row.get_idx(8)?,
                    md5sum: row.get_idx(9)?,
                })
            })
            .collect();

        let gsum_list: Vec<_> = map_result(gsum_list)?;

        Ok(GarminSummaryList::from_vec(gsum_list).with_pool(&pool))
    }

    pub fn dump_summary_to_avro(self, output_filename: &str) -> Result<(), Error> {
        let garmin_summary_avro_schema = GARMIN_SUMMARY_AVRO_SCHEMA;
        let schema = Schema::parse_str(&garmin_summary_avro_schema)?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer.extend_ser(self.summary_list)?;
        writer.flush()?;

        Ok(())
    }

    pub fn write_summary_to_avro_files(&self, summary_cache_dir: &str) -> Result<(), Error> {
        let results: Vec<_> = self
            .summary_list
            .par_iter()
            .map(|gsum| {
                let summary_avro_fname =
                    format!("{}/{}.summary.avro", &summary_cache_dir, &gsum.filename);
                let single_summary = GarminSummaryList::from_vec(vec![gsum.clone()]);
                single_summary.dump_summary_to_avro(&summary_avro_fname)
            })
            .collect();

        map_result(results)?;
        Ok(())
    }

    pub fn from_avro_files(summary_cache_dir: &str) -> Result<GarminSummaryList, Error> {
        let path = Path::new(summary_cache_dir);

        let file_list: Vec<String> = get_file_list(&path);

        let results: Vec<_> = file_list
            .par_iter()
            .map(|f| GarminSummaryList::read_summary_from_avro(f))
            .collect();

        let gsum_result_list: Vec<_> = map_result(results)?;

        let gsum_result_list: Vec<_> = gsum_result_list
            .into_iter()
            .map(|g| g.summary_list)
            .flatten()
            .collect();

        Ok(GarminSummaryList::from_vec(gsum_result_list))
    }

    pub fn write_summary_to_postgres(&self) -> Result<(), Error> {
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
        let conn = self.get_pool()?.get()?;

        conn.execute(&create_table_query, &[])?;

        let insert_query = format!("
            INSERT INTO {} (filename, begin_datetime, sport, total_calories, total_distance, total_duration, total_hr_dur, total_hr_dis, md5sum, number_of_items)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1)
        ", temp_table_name);

        let results: Vec<Result<u64, Error>> = self
            .summary_list
            .par_iter()
            .map(|gsum| {
                let conn = self.get_pool()?.get()?;
                Ok(conn.execute(
                    &insert_query,
                    &[
                        &gsum.filename,
                        &gsum.begin_datetime,
                        &gsum.sport,
                        &gsum.total_calories,
                        &gsum.total_distance,
                        &gsum.total_duration,
                        &gsum.total_hr_dur,
                        &gsum.total_hr_dis,
                        &gsum.md5sum,
                    ],
                )?)
            })
            .collect();

        let _: Vec<_> = map_result(results)?;

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

    pub fn dump_summary_from_postgres_to_avro(pool: &PgPool) -> Result<(), Error> {
        let gsum_list = GarminSummaryList::from_pool(pool).read_summary_from_postgres("")?;

        println!("{}", gsum_list.summary_list.len());

        gsum_list.dump_summary_to_avro("garmin_summary.avro")?;

        let gsum_list = GarminSummaryList::read_summary_from_avro("garmin_summary.avro")?;

        println!("{}", gsum_list.summary_list.len());
        Ok(())
    }
}

pub fn get_list_of_files_from_db(
    constraints: &[String],
    pool: &PgPool,
) -> Result<Vec<String>, Error> {
    let constr = if constraints.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", constraints.join(" OR "))
    };

    let query = format!("SELECT filename FROM garmin_summary {}", constr);

    let conn = pool.get()?;

    let results: Vec<_> = conn
        .query(&query, &[])?
        .iter()
        .map(|row| row.get_idx(0))
        .collect();

    map_result(results)
}
