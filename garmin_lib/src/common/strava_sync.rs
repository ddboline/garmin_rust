use chrono::{DateTime, Utc};
use failure::{err_msg, Error};
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::common::pgpool::PgPool;
use crate::utils::iso_8601_datetime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StravaItem {
    #[serde(with = "iso_8601_datetime")]
    pub begin_datetime: DateTime<Utc>,
    pub title: String,
}

pub fn get_strava_id_from_begin_datetime(
    pool: &PgPool,
    begin_datetime: DateTime<Utc>,
) -> Result<Option<(String, String)>, Error> {
    let query = "SELECT strava_id, strava_title FROM strava_id_cache WHERE begin_datetime = $1";

    let mut conn = pool.get()?;
    conn.query(query, &[&begin_datetime])?
        .get(0)
        .map(|row| {
            let id = row.try_get(0)?;
            let title = row.try_get(1)?;
            Ok((id, title))
        })
        .transpose()
}

pub fn get_strava_id_maximum_begin_datetime(pool: &PgPool) -> Result<Option<DateTime<Utc>>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM strava_id_cache";

    let mut conn = pool.get()?;

    conn.query(query, &[])?
        .get(0)
        .map(|row| row.try_get(0).map_err(err_msg))
        .transpose()
}

pub fn get_strava_id_map(pool: &PgPool) -> Result<HashMap<String, StravaItem>, Error> {
    let query = "SELECT strava_id, begin_datetime, strava_title FROM strava_id_cache";
    let mut conn = pool.get()?;
    conn.query(query, &[])?
        .iter()
        .map(|row| {
            let strava_id: String = row.try_get(0)?;
            let begin_datetime: DateTime<Utc> = row.try_get(1)?;
            let strava_title: String = row.try_get(2)?;
            Ok((
                strava_id,
                StravaItem {
                    begin_datetime,
                    title: strava_title,
                },
            ))
        })
        .collect()
}

pub fn get_strava_ids(
    pool: &PgPool,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
) -> Result<HashMap<String, StravaItem>, Error> {
    let mut constraints = Vec::new();
    if let Some(start_date) = start_date {
        constraints.push(format!("begin_datetime >= '{}'", start_date.to_rfc3339()));
    }
    if let Some(end_date) = end_date {
        constraints.push(format!("begin_datetime <= '{}'", end_date.to_rfc3339()));
    }
    let query = format!(
        "SELECT strava_id, begin_datetime, strava_title FROM strava_id_cache {} ORDER BY begin_datetime",
        if constraints.is_empty() {"".to_string()} else {
            format!("WHERE {}", constraints.join(" OR "))
        } ,
    );
    let mut conn = pool.get()?;
    conn.query(query.as_str(), &[])?
        .iter()
        .map(|row| {
            let strava_id: String = row.try_get(0)?;
            let begin_datetime: DateTime<Utc> = row.try_get(1)?;
            let strava_title: String = row.try_get(2)?;
            Ok((
                strava_id,
                StravaItem {
                    begin_datetime,
                    title: strava_title,
                },
            ))
        })
        .collect()
}

pub fn upsert_strava_id<S: BuildHasher>(
    new_items: &HashMap<String, StravaItem, S>,
    pool: &PgPool,
) -> Result<Vec<String>, Error> {
    let strava_id_map = get_strava_id_map(pool)?;

    let (update_items, insert_items): (Vec<_>, Vec<_>) = new_items
        .iter()
        .partition(|(id, _)| strava_id_map.contains_key(*id));

    let update_items: Vec<_> = update_items
        .into_iter()
        .filter_map(|(id, new_item)| {
            strava_id_map.get(id).and_then(|item| {
                if new_item == item {
                    None
                } else {
                    Some((id, new_item))
                }
            })
        })
        .collect();

    let query = "
        UPDATE strava_id_cache SET strava_title=$2 WHERE strava_id=$1
    ";
    debug!("{}", query);
    let items: Result<Vec<_>, Error> = update_items
        .into_par_iter()
        .map(|(key, val)| {
            let mut conn = pool.get()?;
            conn.execute(query, &[&key, &val.title])?;
            Ok(key.to_string())
        })
        .collect();
    let mut output: Vec<_> = items?;

    let query = "
        INSERT INTO strava_id_cache (strava_id, begin_datetime, strava_title)
        VALUES ($1,$2,$3)
    ";
    debug!("{}", query);
    let items: Result<Vec<_>, Error> = insert_items
        .into_par_iter()
        .map(|(key, val)| {
            let mut conn = pool.get()?;
            conn.execute(query, &[&key, &val.begin_datetime, &val.title])?;
            Ok(key.to_string())
        })
        .collect();

    let items: Vec<_> = items?;

    output.extend(items);
    Ok(output)
}
