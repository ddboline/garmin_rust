use chrono::{DateTime, Utc};
use failure::Error;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::common::pgpool::PgPool;
use crate::utils::garmin_util::map_result;
use crate::utils::iso_8601_datetime;
use crate::utils::row_index_trait::RowIndexTrait;

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

    let conn = pool.get()?;
    conn.query(&query, &[&begin_datetime])?
        .iter()
        .nth(0)
        .map(|row| {
            let id = row.get_idx(0)?;
            let title = row.get_idx(1)?;
            Ok((id, title))
        })
        .transpose()
}

pub fn get_strava_id_maximum_begin_datetime(pool: &PgPool) -> Result<Option<DateTime<Utc>>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM strava_id_cache";

    let conn = pool.get()?;

    conn.query(&query, &[])?
        .iter()
        .nth(0)
        .map(|row| row.get_idx(0))
        .transpose()
}

pub fn get_strava_id_map(pool: &PgPool) -> Result<HashMap<String, StravaItem>, Error> {
    let query = "SELECT strava_id, begin_datetime, strava_title FROM strava_id_cache";
    let conn = pool.get()?;
    let strava_id_map: Vec<Result<_, Error>> = conn
        .query(&query, &[])?
        .iter()
        .map(|row| {
            let strava_id: String = row.get_idx(0)?;
            let begin_datetime: DateTime<Utc> = row.get_idx(1)?;
            let strava_title: String = row.get_idx(2)?;
            Ok((
                strava_id,
                StravaItem {
                    begin_datetime,
                    title: strava_title,
                },
            ))
        })
        .collect();

    map_result(strava_id_map)
}

pub fn upsert_strava_id<S: BuildHasher>(
    new_items: &HashMap<String, StravaItem, S>,
    pool: &PgPool,
) -> Result<Vec<String>, Error> {
    let strava_id_map = get_strava_id_map(pool)?;

    let update_items: Vec<_> = new_items
        .iter()
        .filter_map(|(id, new_item)| {
            strava_id_map.get(id).and_then(|item| {
                if new_item != item {
                    Some((id.clone(), new_item.clone()))
                } else {
                    None
                }
            })
        })
        .collect();

    let insert_items: Vec<_> = new_items
        .iter()
        .filter_map(|(id, new_item)| {
            if strava_id_map.contains_key(id) {
                None
            } else {
                Some((id.clone(), new_item.clone()))
            }
        })
        .collect();

    let query = "
        UPDATE strava_id_cache SET strava_title=$2 WHERE strava_id=$1
    ";
    debug!("{}", query);
    let items: Vec<_> = update_items
        .into_par_iter()
        .map(|(key, val)| {
            let conn = pool.get()?;
            conn.execute(query, &[&key, &val.title])?;
            Ok(key.clone())
        })
        .collect();
    let mut output: Vec<String> = map_result(items)?;

    let query = "
        INSERT INTO strava_id_cache (strava_id, begin_datetime, strava_title)
        VALUES ($1,$2,$3)
    ";
    debug!("{}", query);
    let items: Vec<_> = insert_items
        .into_par_iter()
        .map(|(key, val)| {
            let conn = pool.get()?;
            conn.execute(query, &[&key, &val.begin_datetime, &val.title])?;
            Ok(key.clone())
        })
        .collect();
    let items: Vec<_> = map_result(items)?;
    output.extend(items);
    Ok(output)
}
