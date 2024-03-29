use anyhow::Error;
use futures::{future::try_join_all, Stream, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::collections::HashMap;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use garmin_lib::{date_time_wrapper::DateTimeWrapper, strava_timezone::StravaTimeZone};
use garmin_utils::{pgpool::PgPool, sport_types, sport_types::SportTypes};

use crate::garmin_summary::GarminSummary;

#[derive(Serialize, Deserialize, FromSqlRow, Debug, Clone, PartialEq)]
pub struct StravaActivity {
    pub name: StackString,
    pub start_date: DateTimeWrapper,
    pub id: i64,
    pub distance: Option<f64>,
    pub moving_time: Option<i64>,
    pub elapsed_time: i64,
    pub total_elevation_gain: Option<f64>,
    pub elev_high: Option<f64>,
    pub elev_low: Option<f64>,
    #[serde(alias = "type", with = "sport_types")]
    pub activity_type: SportTypes,
    pub timezone: StravaTimeZone,
}

impl Default for StravaActivity {
    fn default() -> Self {
        Self {
            name: "".into(),
            start_date: DateTimeWrapper::now(),
            id: -1,
            distance: None,
            moving_time: None,
            elapsed_time: 0,
            total_elevation_gain: None,
            elev_high: None,
            elev_low: None,
            activity_type: SportTypes::None,
            timezone: StravaTimeZone::default(),
        }
    }
}

impl StravaActivity {
    /// # Errors
    /// Return error if db query fails
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = "SELECT * FROM strava_activities";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(start_date) >= $start_date");
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_date) <= $end_date");
            bindings.push(("end_date", d));
        }
        let query = format_sstr!(
            "{query} {c} ORDER BY start_date",
            c = if conditions.is_empty() {
                "".into()
            } else {
                format_sstr!("WHERE {}", conditions.join(" AND "))
            }
        );
        let mut query_bindings: Vec<_> =
            bindings.iter().map(|(k, v)| (*k, v as Parameter)).collect();
        query_bindings.shrink_to_fit();
        debug!("query:\n{}", query);
        let query = query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_begin_datetime(
        pool: &PgPool,
        begin_datetime: OffsetDateTime,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM strava_activities WHERE start_date=$start_date",
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
            "SELECT * FROM strava_activities WHERE summary_id=$summary_id",
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
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
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
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    #[allow(clippy::manual_filter_map)]
    pub async fn upsert_activities(
        activities: &[Self],
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        let mut output = Vec::new();
        let mut existing_activities: HashMap<_, _> = Self::read_from_db(pool, None, None)
            .await?
            .map_ok(|activity| (activity.id, activity))
            .try_collect()
            .await?;
        existing_activities.shrink_to_fit();

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
                    let id_str = StackString::from_display(activity.id);
                    Ok(id_str)
                }
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);

        let futures = insert_items.into_iter().map(|activity| {
            let pool = pool.clone();
            async move {
                activity.insert_into_db(&pool).await?;
                let id_str = StackString::from_display(activity.id);
                Ok(id_str)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        output.extend_from_slice(&results?);
        output.shrink_to_fit();

        Ok(output)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn fix_summary_id_in_db(pool: &PgPool) -> Result<(), Error> {
        let query = "
            UPDATE strava_activities SET summary_id = (
                SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_date
            )
            WHERE summary_id IS NULL
        ";
        let conn = pool.get().await?;
        conn.execute(query, &[]).await?;
        Ok(())
    }
}

impl From<GarminSummary> for StravaActivity {
    fn from(item: GarminSummary) -> Self {
        Self {
            name: item.filename,
            start_date: item.begin_datetime,
            distance: Some(item.total_distance),
            elapsed_time: item.total_duration as i64,
            activity_type: item.sport,
            ..Self::default()
        }
    }
}
