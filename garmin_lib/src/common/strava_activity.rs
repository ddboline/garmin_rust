use anyhow::Error;
use chrono::{DateTime, NaiveDate, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::{FromSqlRow, Parameter};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::collections::HashMap;

use crate::{
    common::pgpool::PgPool,
    utils::{
        iso_8601_datetime,
        sport_types::{self, SportTypes},
    },
};

#[derive(Serialize, Deserialize, FromSqlRow, Debug, Clone, PartialEq)]
pub struct StravaActivity {
    pub name: StackString,
    #[serde(with = "iso_8601_datetime")]
    pub start_date: DateTime<Utc>,
    pub id: i64,
    pub distance: Option<f64>,
    pub moving_time: Option<i64>,
    pub elapsed_time: i64,
    pub total_elevation_gain: Option<f64>,
    pub elev_high: Option<f64>,
    pub elev_low: Option<f64>,
    #[serde(rename = "type", with = "sport_types")]
    pub activity_type: SportTypes,
    pub timezone: StackString,
}

impl StravaActivity {
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM strava_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_date) >= $start_date".to_string());
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_date) <= $end_date".to_string());
            bindings.push(("end_date", d));
        }
        let query = format!(
            "{} {} ORDER BY start_date",
            query,
            if conditions.is_empty() {
                "".to_string()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            }
        );
        let query_bindings: Vec<_> = bindings.iter().map(|(k, v)| (*k, v as Parameter)).collect();
        debug!("query:\n{}", query);
        let query = postgres_query::query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .par_iter()
            .map(|r| Self::from_row(r).map_err(Into::into))
            .collect()
    }

    pub async fn get_by_begin_datetime(
        pool: &PgPool,
        begin_datetime: DateTime<Utc>,
    ) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM strava_activities WHERE start_date=$start_date",
            start_date = begin_datetime,
        );
        let conn = pool.get().await?;
        let activity: Option<StravaActivity> = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| StravaActivity::from_row(&row))
            .transpose()?;
        Ok(activity)
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
                INSERT INTO strava_activities (
                    id,name,start_date,distance,moving_time,elapsed_time,
                    total_elevation_gain,elev_high,elev_low,activity_type,timezone
                )
                VALUES (
                    $id,$name,$start_date,$distance,$moving_time,$elapsed_time,
                    $total_elevation_gain,$elev_high,$elev_low,$activity_type,$timezone
                )",
            id = self.id,
            name = self.name,
            start_date = self.start_date,
            distance = self.distance,
            moving_time = self.moving_time,
            elapsed_time = self.elapsed_time,
            total_elevation_gain = self.total_elevation_gain,
            elev_high = self.elev_high,
            elev_low = self.elev_low,
            activity_type = self.activity_type,
            timezone = self.timezone,
        );

        let conn = pool.get().await?;

        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
                UPDATE strava_activities SET
                    name=$name,start_date=$start_date,distance=$distance,moving_time=$moving_time,
                    elapsed_time=$elapsed_time,total_elevation_gain=$total_elevation_gain,
                    elev_high=$elev_high,elev_low=$elev_low,activity_type=$activity_type,
                    timezone=$timezone
                WHERE id=$id
            ",
            id = self.id,
            name = self.name,
            start_date = self.start_date,
            distance = self.distance,
            moving_time = self.moving_time,
            elapsed_time = self.elapsed_time,
            total_elevation_gain = self.total_elevation_gain,
            elev_high = self.elev_high,
            elev_low = self.elev_low,
            activity_type = self.activity_type,
            timezone = self.timezone,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters()).await?;
        Ok(())
    }

    pub async fn upsert_activities(
        activities: &[Self],
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        let mut output = Vec::new();
        let existing_activities: HashMap<_, _> = Self::read_from_db(pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.id, activity))
            .collect();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.id));

        let futures = update_items
            .into_iter()
            .filter(|activity| {
                if let Some(existing_activity) = existing_activities.get(&activity.id) {
                    if activity != &existing_activity {
                        return true;
                    }
                }
                false
            })
            .map(|activity| {
                let pool = pool.clone();
                async move {
                    activity.update_db(&pool).await?;
                    Ok(activity.id.to_string().into())
                }
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(activity.id.to_string().into())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        Ok(output)
    }
}
