use anyhow::Error;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
use serde::{Deserialize, Deserializer, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fmt::Write, fs::File, path::Path};

use super::{garmin_config::GarminConfig, pgpool::PgPool};

#[derive(Serialize, Deserialize, Debug, FromSqlRow)]
pub struct GarminConnectActivity {
    #[serde(alias = "activityId")]
    pub activity_id: i64,
    #[serde(alias = "activityName")]
    pub activity_name: Option<StackString>,
    pub description: Option<StackString>,
    #[serde(alias = "startTimeGMT", deserialize_with = "deserialize_start_time")]
    pub start_time_gmt: DateTime<Utc>,
    pub distance: Option<f64>,
    pub duration: f64,
    #[serde(alias = "elapsedDuration")]
    pub elapsed_duration: Option<f64>,
    #[serde(alias = "movingDuration")]
    pub moving_duration: Option<f64>,
    pub steps: Option<i64>,
    pub calories: Option<f64>,
    #[serde(alias = "averageHR")]
    pub average_hr: Option<f64>,
    #[serde(alias = "maxHR")]
    pub max_hr: Option<f64>,
}

impl GarminConnectActivity {
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM garmin_connect_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_time_gmt) >= $start_date");
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_time_gmt) <= $end_date");
            bindings.push(("end_date", d));
        }
        let query = format_sstr!(
            "{query} {cond} ORDER BY start_time_gmt",
            cond = if conditions.is_empty() {
                "".into()
            } else {
                format_sstr!("WHERE {}", conditions.join(" AND "))
            }
        );
        let query_bindings: Vec<_> = bindings.iter().map(|(k, v)| (*k, v as Parameter)).collect();
        debug!("query:\n{}", query);
        let query = query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn get_by_begin_datetime(
        pool: &PgPool,
        begin_datetime: DateTime<Utc>,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM garmin_connect_activities WHERE start_time_gmt=$start_date",
            start_date = begin_datetime,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn get_from_summary_id(
        pool: &PgPool,
        summary_id: i32,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM garmin_connect_activities WHERE summary_id = $summary_id",
            summary_id = summary_id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                INSERT INTO garmin_connect_activities (
                    activity_id,activity_name,description,start_time_gmt,distance,duration,
                    elapsed_duration,moving_duration,steps,calories,average_hr,max_hr
                )
                VALUES (
                    $activity_id,$activity_name,$description,$start_time_gmt,$distance,$duration,
                    $elapsed_duration,$moving_duration,$steps,$calories,$average_hr,$max_hr
                )",
            activity_id = self.activity_id,
            activity_name = self.activity_name,
            description = self.description,
            start_time_gmt = self.start_time_gmt,
            distance = self.distance,
            duration = self.duration,
            elapsed_duration = self.elapsed_duration,
            moving_duration = self.moving_duration,
            steps = self.steps,
            calories = self.calories,
            average_hr = self.average_hr,
            max_hr = self.max_hr,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                UPDATE garmin_connect_activities SET
                    activity_name=$activity_name,description=$description,
                    start_time_gmt=$start_time_gmt,distance=$distance,duration=$duration,
                    elapsed_duration=$elapsed_duration,moving_duration=$moving_duration,
                    steps=$steps,calories=$calories,average_hr=$average_hr,max_hr=$max_hr
                WHERE activity_id=$activity_id
            ",
            activity_id = self.activity_id,
            activity_name = self.activity_name,
            description = self.description,
            start_time_gmt = self.start_time_gmt,
            distance = self.distance,
            duration = self.duration,
            elapsed_duration = self.elapsed_duration,
            moving_duration = self.moving_duration,
            steps = self.steps,
            calories = self.calories,
            average_hr = self.average_hr,
            max_hr = self.max_hr,
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
            .map(|activity| (activity.activity_id, activity))
            .collect();

        let (update_items, insert_items): (Vec<_>, Vec<_>) = activities
            .iter()
            .partition(|activity| existing_activities.contains_key(&activity.activity_id));

        let futures = update_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.update_db(&pool).await?;
                let activity_str = StackString::from_display(activity.activity_id);
                Ok(activity_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                let activity_str = StackString::from_display(activity.activity_id);
                Ok(activity_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);
        Ok(output)
    }

    pub async fn merge_new_activities(
        new_activities: Vec<Self>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let activities: HashMap<_, _> = GarminConnectActivity::read_from_db(pool, None, None)
            .await?
            .into_iter()
            .map(|activity| (activity.activity_id, activity))
            .collect();

        #[allow(clippy::manual_filter_map)]
        let futures = new_activities
            .into_iter()
            .filter(|activity| !activities.contains_key(&activity.activity_id))
            .map(|activity| async move {
                activity.insert_into_db(pool).await?;
                Ok(activity)
            });
        try_join_all(futures).await
    }

    pub async fn fix_summary_id_in_db(pool: &PgPool) -> Result<(), Error> {
        let query = "
            UPDATE garmin_connect_activities SET summary_id = (
                SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time_gmt
            )
            WHERE summary_id IS NULL
        ";
        let conn = pool.get().await?;
        conn.execute(query, &[]).await?;
        Ok(())
    }
}

pub async fn import_garmin_connect_activity_json_file(filename: &Path) -> Result<(), Error> {
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(&config.pgurl);

    let activities = serde_json::from_reader(File::open(&filename)?)?;
    GarminConnectActivity::merge_new_activities(activities, &pool).await?;
    Ok(())
}

pub fn deserialize_start_time<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if let Ok(dt) = s.parse() {
        Ok(dt)
    } else {
        NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
            .map(|datetime| DateTime::from_utc(datetime, Utc))
            .map_err(serde::de::Error::custom)
    }
}
