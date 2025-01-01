use anyhow::Error;
use futures::future::try_join_all;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::collections::HashMap;
use time::Date;
use uuid::Uuid;

use garmin_lib::date_time_wrapper::DateTimeWrapper;

use garmin_utils::pgpool::PgPool;

#[derive(Serialize, Deserialize, Clone, Debug, FromSqlRow, PartialEq)]
pub struct FitbitActivity {
    #[serde(alias = "logType")]
    pub log_type: StackString,
    #[serde(alias = "startTime")]
    pub start_time: DateTimeWrapper,
    #[serde(alias = "tcxLink")]
    pub tcx_link: Option<StackString>,
    #[serde(alias = "activityTypeId")]
    pub activity_type_id: Option<i64>,
    #[serde(alias = "activityName")]
    pub activity_name: Option<StackString>,
    pub duration: i64,
    pub distance: Option<f64>,
    #[serde(alias = "distanceUnit")]
    pub distance_unit: Option<StackString>,
    pub steps: Option<i64>,
    #[serde(alias = "logId")]
    pub log_id: i64,
}

impl FitbitActivity {
    fn get_fitbit_activity_query<'a>(
        select_str: &'a str,
        start_date: Option<&'a Date>,
        end_date: Option<&'a Date>,
        offset: Option<usize>,
        limit: Option<usize>,
        order_str: &'a str,
    ) -> Result<Query<'a>, PqError> {
        let mut conditions = Vec::new();
        let mut query_bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_time) >= $start_date");
            query_bindings.push(("start_date", d as Parameter));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_time) <= $end_date");
            query_bindings.push(("end_date", d as Parameter));
        }
        let mut query = format_sstr!(
            "SELECT {select_str} FROM fitbit_activities {} {order_str}",
            if conditions.is_empty() {
                "".into()
            } else {
                format_sstr!("WHERE {}", conditions.join(" AND "))
            }
        );
        if let Some(offset) = &offset {
            query.push_str(&format_sstr!(" OFFSET {offset}"));
        }
        if let Some(limit) = &limit {
            query.push_str(&format_sstr!(" LIMIT {limit}"));
        }
        debug!("query:\n{}", query);
        query_dyn!(&query, ..query_bindings)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Vec<Self>, Error> {
        let query = Self::get_fitbit_activity_query(
            "*",
            start_date.as_ref(),
            end_date.as_ref(),
            offset,
            limit,
            "ORDER BY start_time",
        )?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_total(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<usize, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }

        let query =
            Self::get_fitbit_activity_query("count(*)", start_date.as_ref(), end_date.as_ref(), None, None, "")?;
        let conn = pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;

        Ok(count.count.try_into()?)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_id(pool: &PgPool, id: i64) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM fitbit_activities WHERE log_id=$id", id = id);
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_from_summary_id(
        pool: &PgPool,
        summary_id: Uuid,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM fitbit_activities WHERE summary_id = $summary_id LIMIT 1",
            summary_id = summary_id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn delete_from_db(self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "DELETE FROM fitbit_activities WHERE log_id=$id",
            id = self.log_id
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
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

    /// # Errors
    /// Return error if db query fails
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

    /// # Errors
    /// Return error if db query fails
    pub async fn upsert_activities(
        activities: &[Self],
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        let mut output = Vec::new();
        let mut existing_activities: HashMap<_, _> =
            Self::read_from_db(pool, None, None, None, None)
                .await?
                .into_iter()
                .map(|activity| (activity.log_id, activity))
                .collect();
        existing_activities.shrink_to_fit();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.log_id));

        let futures = update_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.update_db(&pool).await?;
                let activity_str = StackString::from_display(activity.log_id);
                Ok(activity_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                let activity_str = StackString::from_display(activity.log_id);
                Ok(activity_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        Ok(output)
    }

    /// # Errors
    /// Return error if db query fails
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
