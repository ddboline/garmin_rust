use avro_rs::{Codec, Schema, Writer};
use chrono::{DateTime, Utc};
use failure::{err_msg, format_err, Error};
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{stdout, Write};
use std::path::Path;

use super::garmin_correction_lap::GarminCorrectionLap;
use super::garmin_file::GarminFile;
use super::pgpool::PgPool;
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::utils::garmin_util::{generate_random_string, get_file_list, get_md5sum};
use crate::utils::iso_8601_datetime::{self, convert_datetime_to_str, sentinel_datetime};

use crate::utils::sport_types::{self, SportTypes};

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
            {"name": "md5sum", "type": "string"}
        ]
    }
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminSummary {
    pub filename: String,
    #[serde(with = "iso_8601_datetime")]
    pub begin_datetime: DateTime<Utc>,
    #[serde(with = "sport_types")]
    pub sport: SportTypes,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub md5sum: String,
}

impl GarminSummary {
    pub fn new(gfile: &GarminFile, md5sum: &str) -> GarminSummary {
        GarminSummary {
            filename: gfile.filename.to_string(),
            begin_datetime: gfile.begin_datetime,
            sport: gfile.sport,
            total_calories: gfile.total_calories,
            total_distance: gfile.total_distance,
            total_duration: gfile.total_duration,
            total_hr_dur: gfile.total_hr_dur,
            total_hr_dis: gfile.total_hr_dis,
            md5sum: md5sum.to_string(),
        }
    }

    pub fn process_single_gps_file(
        filename: &str,
        cache_dir: &str,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummary, Error> {
        let cache_file = format!(
            "{}/{}.avro",
            cache_dir,
            filename
                .split('/')
                .last()
                .ok_or_else(|| format_err!("Failed to split filename {}", filename))?
        );

        writeln!(stdout().lock(), "Get md5sum {} ", filename)?;
        let md5sum = get_md5sum(&filename)?;

        writeln!(stdout().lock(), "{} Found md5sum {} ", filename, md5sum)?;
        let gfile = GarminParse::new().with_file(&filename, &corr_map)?;

        match gfile.laps.get(0) {
            Some(l) if l.lap_start == sentinel_datetime() => {
                return Err(format_err!("{} has empty lap start?", &gfile.filename));
            }
            Some(_) => (),
            None => return Err(format_err!("{} has no laps?", gfile.filename)),
        };
        gfile.dump_avro(&cache_file)?;
        writeln!(
            stdout().lock(),
            "{} Found md5sum {} success",
            filename,
            md5sum
        )?;
        Ok(GarminSummary::new(&gfile, &md5sum))
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
            "md5sum",
        ];
        let vals = vec![
            self.filename.to_string(),
            convert_datetime_to_str(self.begin_datetime),
            self.sport.to_string(),
            self.total_calories.to_string(),
            self.total_distance.to_string(),
            self.total_duration.to_string(),
            self.total_hr_dur.to_string(),
            self.total_hr_dis.to_string(),
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
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let path = Path::new(gps_dir);

        let gsum_result_list: Result<Vec<_>, Error> = get_file_list(&path)
            .into_par_iter()
            .map(|input_file| {
                writeln!(stdout().lock(), "Process {}", &input_file)?;
                let cache_file = format!(
                    "{}/{}.avro",
                    cache_dir,
                    input_file
                        .split('/')
                        .last()
                        .ok_or_else(|| format_err!("Failed to split input_file {}", input_file))?
                );
                let md5sum = get_md5sum(&input_file)?;
                let gfile = GarminParse::new().with_file(&input_file, &corr_map)?;
                match gfile.laps.get(0) {
                    Some(l) if l.lap_start == sentinel_datetime() => {
                        return Err(format_err!(
                            "{} {} has empty lap start?",
                            &input_file,
                            &gfile.filename
                        ));
                    }
                    Some(_) => (),
                    None => {
                        return Err(format_err!(
                            "{} {} has no laps?",
                            &input_file,
                            &gfile.filename
                        ));
                    }
                };
                gfile.dump_avro(&cache_file)?;
                Ok(GarminSummary::new(&gfile, &md5sum))
            })
            .collect();

        Ok(GarminSummaryList::from_vec(gsum_result_list?))
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
                   md5sum
            FROM garmin_summary
            {}",
            where_str
        );
        let pool = self.get_pool()?;
        let mut conn = pool.get()?;

        let gsum_list: Result<Vec<_>, Error> = conn
            .query(query.as_str(), &[])?
            .iter()
            .map(|row| {
                let sport: String = row.try_get(2)?;
                Ok(GarminSummary {
                    filename: row.try_get(0)?,
                    begin_datetime: row.try_get(1)?,
                    sport: sport.parse()?,
                    total_calories: row.try_get(3)?,
                    total_distance: row.try_get(4)?,
                    total_duration: row.try_get(5)?,
                    total_hr_dur: row.try_get(6)?,
                    total_hr_dis: row.try_get(7)?,
                    md5sum: row.try_get(8)?,
                })
            })
            .collect();

        Ok(GarminSummaryList::from_vec(gsum_list?).with_pool(&pool))
    }

    pub fn dump_summary_to_avro(self, output_filename: &str) -> Result<(), Error> {
        let schema = Schema::parse_str(GARMIN_SUMMARY_AVRO_SCHEMA)?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer.extend_ser(self.summary_list)?;
        writer.flush().map(|_| ())
    }

    pub fn write_summary_to_avro_files(&self, summary_cache_dir: &str) -> Result<(), Error> {
        self.summary_list
            .par_iter()
            .map(|gsum| {
                let summary_avro_fname =
                    format!("{}/{}.summary.avro", &summary_cache_dir, &gsum.filename);
                let single_summary = GarminSummaryList::from_vec(vec![gsum.clone()]);
                single_summary.dump_summary_to_avro(&summary_avro_fname)
            })
            .collect()
    }

    pub fn write_summary_to_postgres(&self) -> Result<(), Error> {
        let rand_str = generate_random_string(8);

        let temp_table_name = format!("garmin_summary_{}", rand_str);

        let create_table_query = format!(
            "CREATE TABLE {} (
                filename text NOT NULL PRIMARY KEY,
                begin_datetime TIMESTAMP WITH TIME ZONE NOT NULL,
                sport varchar(12),
                total_calories integer,
                total_distance double precision,
                total_duration double precision,
                total_hr_dur double precision,
                total_hr_dis double precision,
                md5sum varchar(32)
            );",
            temp_table_name
        );
        let mut conn = self.get_pool()?.get()?;

        conn.execute(create_table_query.as_str(), &[])?;

        let insert_query = format!(
            "
            INSERT INTO {} (
                filename, begin_datetime, sport, total_calories, total_distance, total_duration,
                total_hr_dur, total_hr_dis, md5sum
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ",
            temp_table_name
        );

        let results: Result<Vec<u64>, Error> = self
            .summary_list
            .par_iter()
            .map(|gsum| {
                let mut conn = self.get_pool()?.get()?;
                Ok(conn.execute(
                    insert_query.as_str(),
                    &[
                        &gsum.filename,
                        &gsum.begin_datetime,
                        &gsum.sport.to_string(),
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
        results?;

        let insert_query = format!(
            "
            INSERT INTO garmin_summary (
                filename, begin_datetime, sport, total_calories, total_distance, total_duration,
                total_hr_dur, total_hr_dis, md5sum
            )
            SELECT b.filename, b.begin_datetime, b.sport, b.total_calories, b.total_distance,
                   b.total_duration, b.total_hr_dur, b.total_hr_dis, b.md5sum
            FROM {} b
            WHERE b.filename not in (select filename from garmin_summary)
        ",
            temp_table_name
        );

        let update_query = format!(
            "
            UPDATE garmin_summary a
            SET (
                begin_datetime,sport,total_calories,total_distance,total_duration,total_hr_dur,
                total_hr_dis,md5sum
            ) = (b.begin_datetime,b.sport,b.total_calories,b.total_distance,b.total_duration,
                 b.total_hr_dur,b.total_hr_dis,b.md5sum
            )
            FROM {} b
            WHERE a.filename = b.filename
        ",
            temp_table_name
        );

        let drop_table_query = format!("DROP TABLE {}", temp_table_name);

        conn.execute(insert_query.as_str(), &[])?;
        conn.execute(update_query.as_str(), &[])?;
        conn.execute(drop_table_query.as_str(), &[])
            .map(|_| ())
            .map_err(err_msg)
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

    debug!("{}", query);

    let mut conn = pool.get()?;

    conn.query(query.as_str(), &[])?
        .iter()
        .map(|row| row.try_get(0).map_err(err_msg))
        .collect()
}

pub fn get_maximum_begin_datetime(pool: &PgPool) -> Result<Option<DateTime<Utc>>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM garmin_summary";

    let mut conn = pool.get()?;

    conn.query(query, &[])?
        .get(0)
        .map(|row| row.try_get(0).map_err(err_msg))
        .transpose()
}
