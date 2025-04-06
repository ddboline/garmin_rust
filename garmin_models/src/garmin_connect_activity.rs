use futures::{future::try_join_all, Stream, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Deserializer, Serialize};
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fs::File, path::Path};
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
    PrimitiveDateTime,
};
use uuid::Uuid;

use garmin_lib::{date_time_wrapper::DateTimeWrapper, errors::GarminError as Error};

use garmin_utils::pgpool::PgPool;

use garmin_lib::garmin_config::GarminConfig;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GarminConnectSocialProfile {
    pub id: u64,
    #[serde(rename = "displayName")]
    pub display_name: StackString,
    #[serde(rename = "profileId")]
    pub profile_id: u64,
    #[serde(rename = "garminGUID")]
    pub garmin_guid: Uuid,
    #[serde(rename = "fullName")]
    pub full_name: StackString,
    #[serde(rename = "userName")]
    pub username: StackString,
    pub location: StackString,
}

#[derive(Serialize, Deserialize, Debug, FromSqlRow, PartialEq, Clone)]
pub struct GarminConnectActivity {
    #[serde(alias = "activityId")]
    pub activity_id: i64,
    #[serde(alias = "activityName")]
    pub activity_name: Option<StackString>,
    pub description: Option<StackString>,
    #[serde(alias = "startTimeGMT", deserialize_with = "deserialize_start_time")]
    pub start_time_gmt: DateTimeWrapper,
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
    fn garmin_connect_activity_query<'a>(
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
            conditions.push("date(start_time_gmt) >= $start_date");
            query_bindings.push(("start_date", d as Parameter));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_time_gmt) <= $end_date");
            query_bindings.push(("end_date", d as Parameter));
        }
        let mut query = format_sstr!(
            "SELECT {select_str} FROM garmin_connect_activities {cond} {order_str}",
            cond = if conditions.is_empty() {
                "".into()
            } else {
                format_sstr!("WHERE {}", conditions.join(" AND "))
            }
        );
        if let Some(offset) = offset {
            query.push_str(&format_sstr!(" OFFSET {offset}"));
        }
        if let Some(limit) = limit {
            query.push_str(&format_sstr!(" LIMIT {limit}"));
        }
        debug!("query:\n{query}",);
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
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = Self::garmin_connect_activity_query(
            "*",
            start_date.as_ref(),
            end_date.as_ref(),
            offset,
            limit,
            "ORDER BY start_time_gmt",
        )?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
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

        let query = Self::garmin_connect_activity_query(
            "count(*)",
            start_date.as_ref(),
            end_date.as_ref(),
            None,
            None,
            "",
        )?;
        let conn = pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;

        Ok(count.count.try_into()?)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_begin_datetime(
        pool: &PgPool,
        begin_datetime: OffsetDateTime,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM garmin_connect_activities WHERE start_time_gmt=$start_date",
            start_date = begin_datetime,
        );
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
            "SELECT * FROM garmin_connect_activities WHERE summary_id = $summary_id",
            summary_id = summary_id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
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

    /// # Errors
    /// Return error if db query fails
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
                .map_ok(|activity| (activity.activity_id, activity))
                .try_collect()
                .await?;
        existing_activities.shrink_to_fit();

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

    /// # Errors
    /// Return error if db query fails
    pub async fn merge_new_activities(
        new_activities: Vec<Self>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let mut activities: HashMap<_, _> =
            GarminConnectActivity::read_from_db(pool, None, None, None, None)
                .await?
                .map_ok(|activity| (activity.activity_id, activity))
                .try_collect()
                .await?;
        activities.shrink_to_fit();

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

    /// # Errors
    /// Return error if db query fails
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

    /// # Errors
    /// Return error if db query fails
    pub async fn activities_to_download(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            "
            SELECT gca.*
            FROM garmin_connect_activities gca
            LEFT JOIN garmin_summary gs ON gs.id = gca.summary_id
            WHERE gs.id IS NULL
            LIMIT 10
        "
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }
}

/// # Errors
/// Return error if serialization fails
pub async fn import_garmin_connect_activity_json_file(filename: &Path) -> Result<(), Error> {
    let config = GarminConfig::get_config(None)?;
    let pool = PgPool::new(&config.pgurl)?;
    if !filename.exists() {
        return Err(Error::CustomError(format_sstr!(
            "file {filename:?} does not exist"
        )));
    }
    let activities = serde_json::from_reader(File::open(filename)?)?;
    GarminConnectActivity::merge_new_activities(activities, &pool).await?;
    Ok(())
}

/// # Errors
/// Return error if deserialize fails
pub fn deserialize_start_time<'de, D>(deserializer: D) -> Result<DateTimeWrapper, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;

    if let Ok(dt) = OffsetDateTime::parse(&s, &Rfc3339) {
        Ok(dt.into())
    } else if let Ok(d) = PrimitiveDateTime::parse(
        &s,
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"),
    ) {
        Ok(d.assume_utc().into())
    } else {
        PrimitiveDateTime::parse(
            &s,
            format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]"),
        )
        .map(|d| d.assume_utc().into())
        .map_err(serde::de::Error::custom)
    }
}
