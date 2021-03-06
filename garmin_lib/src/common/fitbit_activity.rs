use anyhow::Error;
use chrono::NaiveDate;
use futures::future::try_join_all;
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::collections::HashMap;

use crate::{common::pgpool::PgPool, utils::datetime_wrapper::DateTimeWrapper};

#[derive(Serialize, Deserialize, Clone, Debug, FromSqlRow, Schema)]
pub struct FitbitActivity {
    #[serde(rename = "logType")]
    pub log_type: StackString,
    #[serde(rename = "startTime")]
    pub start_time: DateTimeWrapper,
    #[serde(rename = "tcxLink")]
    pub tcx_link: Option<StackString>,
    #[serde(rename = "activityTypeId")]
    pub activity_type_id: Option<i64>,
    #[serde(rename = "activityName")]
    pub activity_name: Option<StackString>,
    pub duration: i64,
    pub distance: Option<f64>,
    #[serde(rename = "distanceUnit")]
    pub distance_unit: Option<StackString>,
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
        let query = query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn get_by_id(pool: &PgPool, id: i64) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM fitbit_activities WHERE log_id=$id", id = id);
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn get_from_summary_id(
        pool: &PgPool,
        summary_id: i32,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM fitbit_activities WHERE summary_id = $summary_id",
            summary_id = summary_id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn delete_from_db(self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "DELETE FROM fitbit_activities WHERE log_id=$id",
            id = self.log_id
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
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
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
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
        query.execute(&conn).await?;
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
            .map(|activity| (activity.log_id, activity))
            .collect();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.log_id));

        let futures = update_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.update_db(&pool).await?;
                Ok(activity.log_id.to_string().into())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                Ok(activity.log_id.to_string().into())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        Ok(output)
    }

    pub async fn fix_summary_id_in_db(pool: &PgPool) -> Result<(), Error> {
        let query = "
            UPDATE fitbit_activities SET summary_id = (
                SELECT id
                FROM garmin_summary a
                WHERE to_char(a.begin_datetime, 'YYYY-MM-DD HH24:MI')
                        = to_char(start_time, 'YYYY-MM-DD HH24:MI')
            )
            WHERE summary_id IS NULL
        ";
        let conn = pool.get().await?;
        conn.execute(query, &[]).await?;
        Ok(())
    }
}
