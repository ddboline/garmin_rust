use anyhow::Error;
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::BuildHasher};

use crate::{
    common::pgpool::PgPool,
    utils::{iso_8601_datetime, stack_string::StackString},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StravaItem {
    #[serde(with = "iso_8601_datetime")]
    pub begin_datetime: DateTime<Utc>,
    pub title: StackString,
}

pub async fn get_strava_id_from_begin_datetime(
    pool: &PgPool,
    begin_datetime: DateTime<Utc>,
) -> Result<Option<(String, String)>, Error> {
    let query = "SELECT strava_id, strava_title FROM strava_id_cache WHERE begin_datetime = $1";

    let conn = pool.get().await?;
    conn.query(query, &[&begin_datetime])
        .await?
        .get(0)
        .map(|row| {
            let id = row.try_get("strava_id")?;
            let title = row.try_get("strava_title")?;
            Ok((id, title))
        })
        .transpose()
}

pub async fn get_strava_id_maximum_begin_datetime(
    pool: &PgPool,
) -> Result<Option<DateTime<Utc>>, Error> {
    let query = "SELECT MAX(begin_datetime) FROM strava_id_cache";

    let conn = pool.get().await?;

    conn.query_opt(query, &[])
        .await?
        .map(|row| row.try_get(0))
        .transpose()
        .map_err(Into::into)
}

#[derive(FromSqlRow)]
struct StravaIdCache {
    strava_id: StackString,
    begin_datetime: DateTime<Utc>,
    strava_title: StackString,
}

pub async fn get_strava_id_map(pool: &PgPool) -> Result<HashMap<StackString, StravaItem>, Error> {
    let query = "SELECT strava_id, begin_datetime, strava_title FROM strava_id_cache";
    let conn = pool.get().await?;
    conn.query(query, &[])
        .await?
        .iter()
        .map(|row| {
            let c = StravaIdCache::from_row(row)?;
            Ok((
                c.strava_id,
                StravaItem {
                    begin_datetime: c.begin_datetime,
                    title: c.strava_title,
                },
            ))
        })
        .collect()
}

pub async fn get_strava_ids(
    pool: &PgPool,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
) -> Result<HashMap<StackString, StravaItem>, Error> {
    let mut constraints = Vec::new();
    if let Some(start_date) = start_date {
        constraints.push(format!("begin_datetime >= '{}'", start_date.to_rfc3339()));
    }
    if let Some(end_date) = end_date {
        constraints.push(format!("begin_datetime <= '{}'", end_date.to_rfc3339()));
    }
    let query = format!(
        "SELECT strava_id, begin_datetime, strava_title FROM strava_id_cache {} ORDER BY \
         begin_datetime",
        if constraints.is_empty() {
            "".to_string()
        } else {
            format!("WHERE {}", constraints.join(" OR "))
        },
    );
    let conn = pool.get().await?;
    conn.query(query.as_str(), &[])
        .await?
        .iter()
        .map(|row| {
            let c = StravaIdCache::from_row(row)?;
            Ok((
                c.strava_id,
                StravaItem {
                    begin_datetime: c.begin_datetime,
                    title: c.strava_title,
                },
            ))
        })
        .collect()
}

pub async fn upsert_strava_id<S: BuildHasher>(
    new_items: &HashMap<StackString, StravaItem, S>,
    pool: &PgPool,
) -> Result<Vec<String>, Error> {
    let strava_id_map = get_strava_id_map(pool).await?;

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
        UPDATE strava_id_cache SET strava_title=$title WHERE strava_id=$id
    ";
    debug!("{}", query);
    debug!("update_items {:?}", update_items);
    let futures = update_items.into_iter().map(|(key, val)| {
        let pool = pool.clone();
        async move {
            let query = postgres_query::query_dyn!(query, title = val.title, id = key)?;
            let conn = pool.get().await?;
            conn.execute(query.sql(), query.parameters()).await?;
            Ok(key.to_string())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    let mut output = results?;

    let query = "
        INSERT INTO strava_id_cache (strava_id, begin_datetime, strava_title)
        VALUES ($id,$datetime,$title)
    ";
    debug!("{}", query);
    debug!("insert_items {:?}", insert_items);
    let futures = insert_items.into_iter().map(|(key, val)| {
        let pool = pool.clone();
        async move {
            let query = postgres_query::query_dyn!(
                query,
                id = key,
                datetime = val.begin_datetime,
                title = val.title
            )?;
            let conn = pool.get().await?;
            conn.execute(query.sql(), query.parameters()).await?;
            Ok(key.to_string())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    output.extend_from_slice(&results?);

    Ok(output)
}
