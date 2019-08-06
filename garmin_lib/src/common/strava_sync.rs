use failure::Error;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use reqwest::{Client, Url};
use std::collections::HashMap;

use crate::common::garmin_config::GarminConfig;
use crate::common::pgpool::PgPool;
use crate::utils::garmin_util::map_result;
use crate::utils::row_index_trait::RowIndexTrait;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StravaItem {
    pub begin_datetime: String,
    pub title: String,
}

pub fn get_strava_id_from_begin_datetime(
    pool: &PgPool,
    begin_datetime: &str,
) -> Result<Option<(String, String)>, Error> {
    let query = format!(
        r#"SELECT strava_id, strava_title FROM strava_id_cache WHERE begin_datetime = '{}'"#,
        begin_datetime
    );

    let conn = pool.get()?;
    conn.query(&query, &[])?
        .iter()
        .nth(0)
        .map(|row| {
            let id = row.get_idx(0)?;
            let title = row.get_idx(1)?;
            Ok((id, title))
        })
        .transpose()
}

pub fn get_strava_id_maximum_begin_datetime(pool: &PgPool) -> Result<Option<String>, Error> {
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
            let begin_datetime: String = row.get_idx(1)?;
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

pub fn upsert_strava_id(
    pool: &PgPool,
    config: &GarminConfig,
    max_datetime: &str,
) -> Result<Vec<String>, Error> {
    let strava_id_map = get_strava_id_map(pool)?;
    let url = Url::parse_with_params(
        &format!("https://{}/strava/activities", &config.domain),
        &[("start_date", max_datetime)],
    )?;
    println!("{}", url.as_str());
    let resp: HashMap<String, StravaItem> = Client::new().get(url).send()?.json()?;

    let update_items: Vec<_> = resp
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

    let insert_items: Vec<_> = resp
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
