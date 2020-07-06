use anyhow::{format_err, Error};
use avro_rs::{Codec, Schema, Writer};
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::FromSqlRow;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, fs::File, path::Path, sync::Arc};

use super::{garmin_correction_lap::GarminCorrectionLap, garmin_file::GarminFile, pgpool::PgPool};
use crate::{
    parsers::garmin_parse::{GarminParse, GarminParseTrait},
    utils::{
        garmin_util::{generate_random_string, get_file_list, get_md5sum},
        iso_8601_datetime::{self, convert_datetime_to_str, sentinel_datetime},
        stack_string::StackString,
    },
};

use crate::utils::sport_types::SportTypes;

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

#[derive(Debug, Clone, Serialize, Deserialize, FromSqlRow)]
pub struct GarminSummary {
    pub filename: StackString,
    #[serde(with = "iso_8601_datetime")]
    pub begin_datetime: DateTime<Utc>,
    pub sport: SportTypes,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub md5sum: StackString,
}

impl GarminSummary {
    pub fn new(gfile: &GarminFile, md5sum: &str) -> Self {
        Self {
            filename: gfile.filename.clone().into(),
            begin_datetime: gfile.begin_datetime,
            sport: gfile.sport,
            total_calories: gfile.total_calories,
            total_distance: gfile.total_distance,
            total_duration: gfile.total_duration,
            total_hr_dur: gfile.total_hr_dur,
            total_hr_dis: gfile.total_hr_dis,
            md5sum: md5sum.into(),
        }
    }

    pub fn process_single_gps_file(
        filepath: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<Self, Error> {
        let filename = filepath
            .file_name()
            .ok_or_else(|| format_err!("Failed to split filename {:?}", filepath))?
            .to_string_lossy();
        let cache_file = cache_dir.join(filename.as_ref()).with_extension("avro");

        debug!("Get md5sum {} ", filename);
        let md5sum = get_md5sum(&filepath)?;

        debug!("{} Found md5sum {} ", filename, md5sum);
        let gfile = GarminParse::new().with_file(&filepath, &corr_map)?;

        match gfile.laps.get(0) {
            Some(l) if l.lap_start == sentinel_datetime() => {
                return Err(format_err!("{} has empty lap start?", &gfile.filename));
            }
            Some(_) => (),
            None => return Err(format_err!("{} has no laps?", gfile.filename)),
        };
        gfile.dump_avro(&cache_file)?;
        debug!("{:?} Found md5sum {} success", filepath, md5sum);
        Ok(Self::new(&gfile, &md5sum))
    }

    pub fn process_all_gps_files(
        gps_dir: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<Vec<Self>, Error> {
        let path = Path::new(gps_dir);

        get_file_list(&path)
            .into_par_iter()
            .map(|input_file| {
                debug!("Process {:?}", &input_file);
                let cache_file = cache_dir
                    .join(input_file.file_name().ok_or_else(|| {
                        format_err!("Failed to split input_file {:?}", input_file)
                    })?)
                    .with_extension("avro");
                let md5sum = get_md5sum(&input_file)?;
                let gfile = GarminParse::new().with_file(&input_file, &corr_map)?;
                match gfile.laps.get(0) {
                    Some(l) if l.lap_start == sentinel_datetime() => {
                        return Err(format_err!(
                            "{:?} {:?} has empty lap start?",
                            &input_file,
                            &gfile.filename
                        ));
                    }
                    Some(_) => (),
                    None => {
                        return Err(format_err!(
                            "{:?} {:?} has no laps?",
                            &input_file,
                            &gfile.filename
                        ));
                    }
                };
                gfile.dump_avro(&cache_file)?;
                Ok(GarminSummary::new(&gfile, &md5sum))
            })
            .collect()
    }

    pub async fn read_summary_from_postgres(
        pool: &PgPool,
        pattern: &str,
    ) -> Result<Vec<Self>, Error> {
        let where_str = if pattern.is_empty() {
            "".to_string()
        } else {
            format!("WHERE filename like '%{}%'", pattern)
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
        pool.get()
            .await?
            .query(query.as_str(), &[])
            .await?
            .iter()
            .map(|row| GarminSummary::from_row(row).map_err(Into::into))
            .collect()
    }

    pub fn dump_summary_to_avro(
        summary_list: &[&Self],
        output_filename: &Path,
    ) -> Result<(), Error> {
        let schema =
            Schema::parse_str(GARMIN_SUMMARY_AVRO_SCHEMA).map_err(|e| format_err!("{}", e))?;

        let output_file = File::create(output_filename)?;

        let mut writer = Writer::with_codec(&schema, output_file, Codec::Snappy);

        writer
            .extend_ser(summary_list)
            .and_then(|_| writer.flush().map(|_| ()))
            .map_err(|e| format_err!("{}", e))
    }

    pub fn write_summary_to_avro_files(
        summary_list: &[Self],
        summary_cache_dir: &Path,
    ) -> Result<(), Error> {
        summary_list
            .par_iter()
            .map(|gsum| {
                let summary_avro_fname = summary_cache_dir
                    .join(gsum.filename.as_str())
                    .with_extension("summary.avro");
                Self::dump_summary_to_avro(&[gsum], &summary_avro_fname)
            })
            .collect()
    }

    pub async fn write_summary_to_postgres(
        summary_list: &[Self],
        pool: &PgPool,
    ) -> Result<(), Error> {
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
        let conn = pool.get().await?;

        conn.execute(create_table_query.as_str(), &[]).await?;

        let insert_query = Arc::new(format!(
            "
            INSERT INTO {} (
                filename, begin_datetime, sport, total_calories, total_distance, total_duration,
                total_hr_dur, total_hr_dis, md5sum
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ",
            temp_table_name
        ));

        let futures = summary_list.iter().map(|gsum| {
            let pool = pool.clone();
            let insert_query = insert_query.clone();
            async move {
                let conn = pool.get().await?;
                conn.execute(
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
                )
                .await?;
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
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

        conn.execute(insert_query.as_str(), &[]).await?;
        conn.execute(update_query.as_str(), &[]).await?;
        conn.execute(drop_table_query.as_str(), &[])
            .await
            .map(|_| ())
            .map_err(Into::into)
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

pub async fn get_list_of_files_from_db(
    constraints: &str,
    pool: &PgPool,
) -> Result<Vec<StackString>, Error> {
    let constr = if constraints.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", constraints)
    };

    let query = format!("SELECT filename FROM garmin_summary {}", constr);

    debug!("{}", query);

    let conn = pool.get().await?;

    conn.query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| row.try_get("filename").map_err(Into::into))
        .collect()
}

pub async fn get_filename_from_datetime(
    pool: &PgPool,
    begin_datetime: DateTime<Utc>,
) -> Result<Option<StackString>, Error> {
    let query = r#"
        SELECT filename
        FROM garmin_summary
        WHERE begin_datetime = $1
    "#;
    let conn = pool.get().await?;
    conn.query(query, &[&begin_datetime])
        .await?
        .pop()
        .map(|row| {
            let filename: StackString = row.try_get("filename")?;
            Ok(filename)
        })
        .transpose()
}

pub async fn get_list_of_activities_from_db(
    constraints: &str,
    pool: &PgPool,
) -> Result<Vec<(DateTime<Utc>, StackString)>, Error> {
    let constr = if constraints.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", constraints)
    };

    let query = format!(
        "SELECT begin_datetime, filename FROM garmin_summary {}",
        constr
    );

    debug!("{}", query);

    let conn = pool.get().await?;

    conn.query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| {
            let begin_datetime = row.try_get("begin_datetime")?;
            let filename = row.try_get("filename")?;
            Ok((begin_datetime, filename))
        })
        .collect()
}

pub async fn get_maximum_begin_datetime(pool: &PgPool) -> Result<Option<DateTime<Utc>>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM garmin_summary";

    let conn = pool.get().await?;

    conn.query_opt(query, &[])
        .await?
        .map(|row| row.try_get(0))
        .transpose()
        .map_err(Into::into)
}
