use anyhow::{format_err, Error};
use futures::{future::try_join_all, Stream, TryStreamExt};
use itertools::Itertools;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fmt, path::Path, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    parsers::garmin_parse::{GarminParse, GarminParseTrait},
    utils::{
        date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
        garmin_util::{generate_random_string, get_file_list, get_md5sum},
        sport_types::SportTypes,
    },
};

use super::{garmin_correction_lap::GarminCorrectionLap, garmin_file::GarminFile, pgpool::PgPool};

#[derive(Debug, Clone, Serialize, Deserialize, FromSqlRow, PartialEq)]
pub struct GarminSummary {
    pub id: Uuid,
    pub filename: StackString,
    pub begin_datetime: DateTimeWrapper,
    pub sport: SportTypes,
    pub total_calories: i32,
    pub total_distance: f64,
    pub total_duration: f64,
    pub total_hr_dur: f64,
    pub total_hr_dis: f64,
    pub md5sum: StackString,
}

impl GarminSummary {
    #[must_use]
    pub fn new(gfile: &GarminFile, md5sum: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            filename: gfile.filename.clone(),
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

    /// # Errors
    /// Return error if parsing or dumping avro fails
    pub fn process_single_gps_file(
        filepath: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<Self, Error> {
        let filename = filepath
            .file_name()
            .ok_or_else(|| format_err!("Failed to split filename {filepath:?}"))?
            .to_string_lossy();
        let cache_file = cache_dir.join(&format_sstr!("{filename}.avro"));

        debug!("Get md5sum {} ", filename);
        let md5sum = get_md5sum(filepath)?;

        debug!("{} Found md5sum {} ", filename, md5sum);
        let gfile = GarminParse::new().with_file(filepath, corr_map)?;
        let filename = &gfile.filename;
        match gfile.laps.get(0) {
            Some(l) if l.lap_start == DateTimeWrapper::sentinel_datetime() => {
                return Err(format_err!("{filename} has empty lap start?"));
            }
            Some(_) => (),
            None => return Err(format_err!("{filename} has no laps?")),
        };
        gfile.dump_avro(&cache_file)?;
        debug!("{filepath:?} Found md5sum {md5sum} success");
        Ok(Self::new(&gfile, &md5sum))
    }

    /// # Errors
    /// Return error if parsing or dumping avro fails
    pub fn process_all_gps_files(
        gps_dir: &Path,
        cache_dir: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<Vec<Self>, Error> {
        let path = Path::new(gps_dir);

        get_file_list(path)
            .into_par_iter()
            .map(|input_file| {
                debug!("Process {:?}", &input_file);
                let filename = input_file
                    .file_name()
                    .ok_or_else(|| format_err!("Failed to split input_file {input_file:?}"))?
                    .to_string_lossy();
                let cache_file = cache_dir.join(&format_sstr!("{filename}.avro"));
                let md5sum = get_md5sum(&input_file)?;
                let gfile = GarminParse::new().with_file(&input_file, corr_map)?;
                let filename = &gfile.filename;
                match gfile.laps.get(0) {
                    Some(l) if l.lap_start == DateTimeWrapper::sentinel_datetime() => {
                        return Err(format_err!(
                            "{input_file:?} {filename:?} has empty lap start?"
                        ));
                    }
                    Some(_) => (),
                    None => {
                        return Err(format_err!("{input_file:?} {filename:?} has no laps?"));
                    }
                };
                gfile.dump_avro(&cache_file)?;
                Ok(GarminSummary::new(&gfile, &md5sum))
            })
            .collect()
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn read_summary_from_postgres(
        pool: &PgPool,
        pattern: &str,
    ) -> Result<Option<Self>, Error> {
        let where_str = if pattern.is_empty() {
            "".into()
        } else {
            format_sstr!("WHERE filename like '%{pattern}%'")
        };

        let query = format_sstr!(
            "
                SELECT id,
                    filename,
                    begin_datetime,
                    sport,
                    total_calories,
                    total_distance,
                    total_duration,
                    total_hr_dur,
                    total_hr_dis,
                    md5sum
                FROM garmin_summary
                {where_str}
                ORDER BY begin_datetime DESC
                LIMIT 1
            "
        );
        let query = query_dyn!(&query)?;
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_filename(pool: &PgPool, filename: &str) -> Result<Option<Self>, Error> {
        let query = query!(
            "
            SELECT id,
                   filename,
                   begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   total_hr_dur,
                   total_hr_dis,
                   md5sum
            FROM garmin_summary WHERE filename = $filename",
            filename = filename,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_id(pool: &PgPool, id: i32) -> Result<Option<Self>, Error> {
        let query = query!(
            "
            SELECT id,
                   filename,
                   begin_datetime,
                   sport,
                   total_calories,
                   total_distance,
                   total_duration,
                   total_hr_dur,
                   total_hr_dis,
                   md5sum
            FROM garmin_summary WHERE id = $id",
            id = id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn write_summary_to_postgres(
        summary_list: &[Self],
        pool: &PgPool,
    ) -> Result<(), Error> {
        let rand_str = generate_random_string(8);

        let temp_table_name = format_sstr!("garmin_summary_{rand_str}");

        let create_table_query = format_sstr!(
            "CREATE TABLE {temp_table_name} (
                filename text NOT NULL PRIMARY KEY,
                begin_datetime TIMESTAMP WITH TIME ZONE NOT NULL,
                sport varchar(12),
                total_calories integer,
                total_distance double precision,
                total_duration double precision,
                total_hr_dur double precision,
                total_hr_dis double precision,
                md5sum varchar(32)
            );"
        );
        let conn = pool.get().await?;

        conn.execute(create_table_query.as_str(), &[]).await?;

        let insert_query = Arc::new(format_sstr!(
            "
            INSERT INTO {temp_table_name} (
                filename, begin_datetime, sport, total_calories, total_distance, total_duration,
                total_hr_dur, total_hr_dis, md5sum
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "
        ));

        let futures = summary_list.iter().map(|gsum| {
            let pool = pool.clone();
            let insert_query = insert_query.clone();
            async move {
                let conn = pool.get().await?;
                let sport_str = StackString::from_display(gsum.sport);
                conn.execute(
                    insert_query.as_str(),
                    &[
                        &gsum.filename,
                        &gsum.begin_datetime,
                        &sport_str,
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
        let results: Result<Vec<()>, Error> = try_join_all(futures).await;
        results?;

        let insert_query = format_sstr!(
            "
            INSERT INTO garmin_summary (
                filename, begin_datetime, sport, total_calories, total_distance, total_duration,
                total_hr_dur, total_hr_dis, md5sum
            )
            SELECT b.filename, b.begin_datetime, b.sport, b.total_calories, b.total_distance,
                   b.total_duration, b.total_hr_dur, b.total_hr_dis, b.md5sum
            FROM {temp_table_name} b
            WHERE b.filename not in (select filename from garmin_summary)
        "
        );

        let update_query = format_sstr!(
            "
            UPDATE garmin_summary a
            SET (
                begin_datetime,sport,total_calories,total_distance,total_duration,total_hr_dur,
                total_hr_dis,md5sum
            ) = (b.begin_datetime,b.sport,b.total_calories,b.total_distance,b.total_duration,
                 b.total_hr_dur,b.total_hr_dis,b.md5sum
            )
            FROM {temp_table_name} b
            WHERE a.filename = b.filename
        "
        );

        let drop_table_query = format_sstr!("DROP TABLE {temp_table_name}");

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
        let keys = [
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
        let vals = [
            self.filename.clone(),
            convert_datetime_to_str(self.begin_datetime.into()),
            StackString::from_display(self.sport),
            StackString::from_display(self.total_calories),
            StackString::from_display(self.total_distance),
            StackString::from_display(self.total_duration),
            StackString::from_display(self.total_hr_dur),
            StackString::from_display(self.total_hr_dis),
            self.md5sum.clone(),
        ];
        write!(
            f,
            "GarminSummaryTable<{}>",
            keys.iter()
                .zip(vals.iter())
                .map(|(k, v)| { format_sstr!("{k}={v}") })
                .join(",")
        )
    }
}

/// # Errors
/// Return error if db query fails
pub async fn get_list_of_files_from_db(
    constraints: &str,
    pool: &PgPool,
) -> Result<impl Stream<Item = Result<StackString, PqError>>, Error> {
    let constr = if constraints.is_empty() {
        "".into()
    } else {
        format_sstr!("WHERE {constraints}")
    };

    let query = format_sstr!(
        "
            SELECT a.filename
            FROM garmin_summary a
            LEFT JOIN strava_activities b ON a.id = b.summary_id
            {constr}
        "
    );

    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query
        .query_streaming(&conn)
        .await
        .map(|stream| {
            stream.and_then(|row| async move {
                let s: StackString = row.try_get("filename").map_err(PqError::BeginTransaction)?;
                Ok(s)
            })
        })
        .map_err(Into::into)
}

/// # Errors
/// Return error if db query fails
pub async fn get_filename_from_datetime(
    pool: &PgPool,
    begin_datetime: OffsetDateTime,
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

/// # Errors
/// Return error if db query fails
pub async fn get_list_of_activities_from_db(
    constraints: &str,
    pool: &PgPool,
) -> Result<impl Stream<Item = Result<(OffsetDateTime, StackString), PqError>>, Error> {
    let constr = if constraints.is_empty() {
        "".into()
    } else {
        format_sstr!("WHERE {constraints}")
    };

    let query = format_sstr!("SELECT begin_datetime, filename FROM garmin_summary {constr}",);
    debug!("{}", query);
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query
        .query_streaming(&conn)
        .await
        .map(|stream| {
            stream.and_then(|row| async move {
                let begin_datetime = row
                    .try_get("begin_datetime")
                    .map_err(PqError::BeginTransaction)?;
                let filename = row.try_get("filename").map_err(PqError::BeginTransaction)?;
                Ok((begin_datetime, filename))
            })
        })
        .map_err(Into::into)
}

/// # Errors
/// Return error if db query fails
pub async fn get_maximum_begin_datetime(pool: &PgPool) -> Result<Option<OffsetDateTime>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM garmin_summary";

    let conn = pool.get().await?;

    conn.query_opt(query, &[])
        .await?
        .map(|row| row.try_get(0))
        .transpose()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        common::garmin_summary,
        utils::{date_time_wrapper::iso8601::convert_str_to_datetime, sport_types::SportTypes},
    };

    #[test]
    fn test_garmin_file_test_display() {
        let garmin_summary = garmin_summary::GarminSummary {
            id: Uuid::new_v4(),
            filename: "test_file".into(),
            begin_datetime: convert_str_to_datetime("2011-05-07T15:43:07-04:00")
                .unwrap()
                .into(),
            sport: SportTypes::Running,
            total_calories: 15,
            total_distance: 32.0,
            total_duration: 16.0,
            total_hr_dur: 1234.0,
            total_hr_dis: 23456.0,
            md5sum: "asjgpqowiqwe".into(),
        };
        assert_eq!(
            format!("{}", garmin_summary),
            "GarminSummaryTable<filename=test_file,begin_datetime=2011-05-07T19:43:07Z,\
             sport=running,total_calories=15,total_distance=32,total_duration=16,\
             total_hr_dur=1234,total_hr_dis=23456,md5sum=asjgpqowiqwe>"
        );
    }
}
