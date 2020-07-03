use anyhow::Error;
use chrono::{DateTime, NaiveDate, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::{FromSqlRow, Parameter};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::common::pgpool::PgPool;

#[derive(Serialize, Deserialize, Clone, Debug, FromSqlRow)]
pub struct FitbitActivity {
    #[serde(rename = "logType")]
    pub log_type: String,
    #[serde(rename = "startTime")]
    pub start_time: DateTime<Utc>,
    #[serde(rename = "tcxLink")]
    pub tcx_link: Option<String>,
    #[serde(rename = "activityTypeId")]
    pub activity_type_id: Option<i64>,
    #[serde(rename = "activityName")]
    pub activity_name: Option<String>,
    pub duration: i64,
    pub distance: Option<f64>,
    #[serde(rename = "distanceUnit")]
    pub distance_unit: Option<String>,
    pub steps: Option<i64>,
    #[serde(rename = "logId")]
    pub log_id: i64,
}

impl FitbitActivity {
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM fitbit_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_time) >= $start_date".to_string());
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_time) <= $end_date".to_string());
            bindings.push(("end_date", d));
        }
        let query = format!(
            "{} {} ORDER BY start_time",
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

    pub async fn get_by_id(pool: &PgPool, id: i64) -> Result<Option<Self>, Error> {
        let query =
            postgres_query::query!("SELECT * FROM fitbit_activities WHERE log_id=$id", id = id);
        let conn = pool.get().await?;
        let activity = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(activity)
    }

    pub async fn get_by_start_time(
        pool: &PgPool,
        start_time: DateTime<Utc>,
    ) -> Result<Option<Self>, Error> {
        let key = start_time.format("%Y-%m-%d %H:%M").to_string();
        let query = postgres_query::query!(
            "SELECT * FROM fitbit_activities
             WHERE to_char(start_time, 'YYYY-MM-DD HH24:MI') = $key LIMIT 1",
            key = key,
        );
        let conn = pool.get().await?;
        let activity: Option<FitbitActivity> = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| FitbitActivity::from_row(&row))
            .transpose()?;
        Ok(activity)
    }

    pub async fn delete_from_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "DELETE FROM fitbit_activities WHERE log_id=$id",
            id = self.log_id
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters()).await?;
        Ok(())
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
                INSERT INTO fitbit_activities (
                    log_id,log_type,start_time,tcx_link,activity_type_id,activity_name,duration,
                    distance,distance_unit,steps
                )
                VALUES (
                    \
             $log_id,$log_type,$start_time,$tcx_link,$activity_type_id,$activity_name,$duration,
                    $distance,$distance_unit,$steps
                )",
            log_id = self.log_id,
            log_type = self.log_type,
            start_time = self.start_time,
            tcx_link = self.tcx_link,
            activity_type_id = self.activity_type_id,
            activity_name = self.activity_name,
            duration = self.duration,
            distance = self.distance,
            distance_unit = self.distance_unit,
            steps = self.steps,
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
                UPDATE fitbit_activities SET
                    log_type=$log_type,start_time=$start_time,tcx_link=$tcx_link,
                    activity_type_id=$activity_type_id,activity_name=$activity_name,
                    duration=$duration,distance=$distance,distance_unit=$distance_unit,
                    steps=$steps
                WHERE log_id=$log_id
            ",
            log_id = self.log_id,
            log_type = self.log_type,
            start_time = self.start_time,
            tcx_link = self.tcx_link,
            activity_type_id = self.activity_type_id,
            activity_name = self.activity_name,
            duration = self.duration,
            distance = self.distance,
            distance_unit = self.distance_unit,
            steps = self.steps,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters()).await?;
        Ok(())
    }

    pub async fn upsert_activities(
        activities: &[Self],
        pool: &PgPool,
    ) -> Result<Vec<String>, Error> {
        let mut output = Vec::new();
        let existing_activities: HashMap<_, _> = Self::read_from_db(pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.log_id, activity))
            .collect();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.log_id));

        let futures = update_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.update_db(&pool).await?;
                Ok(activity.log_id.to_string())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(activity.log_id.to_string())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        Ok(output)
    }
}
