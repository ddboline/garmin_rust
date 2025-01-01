use anyhow::Error;
use futures::{future::try_join_all, Stream, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Deserializer, Serialize};
use stack_string::{format_sstr, StackString};
use std::collections::HashMap;
use time::{macros::format_description, Date, OffsetDateTime};
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
    fn get_strava_activity_query<'a>(
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
            conditions.push("date(start_date) >= $start_date");
            query_bindings.push(("start_date", d as Parameter));
        }
        if let Some(d) = end_date {
            conditions.push("date(start_date) <= $end_date");
            query_bindings.push(("end_date", d as Parameter));
        }
        let mut query = format_sstr!(
            "SELECT {select_str} FROM strava_activities {} {order_str}",
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
        query_bindings.shrink_to_fit();
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
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = Self::get_strava_activity_query(
            "*",
            start_date.as_ref(),
            end_date.as_ref(),
            offset,
            limit,
            "ORDER BY start_date",
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

        let query = Self::get_strava_activity_query(
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
        let mut existing_activities: HashMap<_, _> =
            Self::read_from_db(pool, None, None, None, None)
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

#[derive(Deserialize, Debug)]
pub struct StravaActivityHarJson {
    pub models: Vec<StravaActivityHarModel>,
}

#[derive(Deserialize, Debug)]
pub struct StravaActivityHarModel {
    pub id: i64,
    pub name: StackString,
    #[serde(with = "sport_types")]
    pub sport_type: SportTypes,
    pub display_type: StackString,
    pub activity_type_display_name: StackString,
    #[serde(deserialize_with = "deserialize_start_time")]
    pub start_time: DateTimeWrapper,
    pub distance_raw: Option<f64>,
    pub moving_time_raw: Option<i64>,
    pub elapsed_time_raw: i64,
    pub elevation_gain_raw: Option<f64>,
}

impl From<StravaActivityHarModel> for StravaActivity {
    fn from(value: StravaActivityHarModel) -> Self {
        Self {
            name: value.name,
            start_date: value.start_time,
            id: value.id,
            distance: value.distance_raw,
            moving_time: value.moving_time_raw,
            elapsed_time: value.elapsed_time_raw,
            total_elevation_gain: value.elevation_gain_raw,
            activity_type: value.sport_type,
            ..StravaActivity::default()
        }
    }
}

/// # Errors
/// Return error if deserialize fails
pub fn deserialize_start_time<'de, D>(deserializer: D) -> Result<DateTimeWrapper, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;

    OffsetDateTime::parse(
        &s,
        format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]+[offset_hour][offset_minute]"
        ),
    )
    .map(Into::into)
    .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::strava_activity::{StravaActivity, StravaActivityHarJson};

    #[test]
    fn test_strava_activity_har_activity_model() -> Result<(), Error> {
        let buf = include_str!("../../tests/data/strava_training_activities.json");
        let js: StravaActivityHarJson = serde_json::from_str(buf)?;
        let activities: Vec<StravaActivity> = js.models.into_iter().map(Into::into).collect();
        assert_eq!(activities.len(), 20);
        Ok(())
    }
}
