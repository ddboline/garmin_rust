use anyhow::Error;
use futures::Stream;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Serialize};
use stack_string::format_sstr;
use statistical::{mean, median, standard_deviation};
use std::convert::TryInto;
use time::Date;
use time_tz::OffsetDateTimeExt;

use garmin_lib::date_time_wrapper::DateTimeWrapper;
use garmin_utils::pgpool::PgPool;

use crate::fitbit_heartrate::FitbitHeartRate;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, FromSqlRow)]
pub struct FitbitStatisticsSummary {
    pub date: Date,
    pub min_heartrate: f64,
    pub max_heartrate: f64,
    pub mean_heartrate: f64,
    pub median_heartrate: f64,
    pub stdev_heartrate: f64,
    pub number_of_entries: i32,
}

impl FitbitStatisticsSummary {
    #[must_use]
    pub fn from_heartrate_values(heartrate_values: &[FitbitHeartRate]) -> Option<Self> {
        let local = DateTimeWrapper::local_tz();
        if heartrate_values.len() < 2 {
            return None;
        }
        let date = heartrate_values[heartrate_values.len() / 2]
            .datetime
            .to_timezone(local)
            .date();
        let min_heartrate = f64::from(heartrate_values.iter().map(|hv| hv.value).min()?);
        let max_heartrate = f64::from(heartrate_values.iter().map(|hv| hv.value).max()?);
        let mut values: Vec<_> = heartrate_values
            .iter()
            .map(|hv| f64::from(hv.value))
            .collect();
        values.shrink_to_fit();
        let mean_heartrate = mean(&values);
        let median_heartrate = median(&values);
        let stdev_heartrate = standard_deviation(&values, Some(mean_heartrate));
        Some(Self {
            date,
            min_heartrate,
            max_heartrate,
            mean_heartrate,
            median_heartrate,
            stdev_heartrate,
            number_of_entries: values.len() as i32,
        })
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn read_entry(date: Date, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
            SELECT * FROM heartrate_statistics_summary WHERE date = $date
        "#,
            date = date
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    fn get_fitbit_statistics_query<'a>(
        select_str: &'a str,
        start_date: Option<&'a Date>,
        end_date: Option<&'a Date>,
        offset: Option<usize>,
        limit: Option<usize>,
        order_str: &'a str,
    ) -> Result<Query<'a>, PqError> {
        let mut conditions = Vec::new();
        let mut query_bindings = Vec::new();
        if let Some(start_date) = start_date {
            conditions.push("date >= $start_date");
            query_bindings.push(("start_date", start_date as Parameter));
        }
        if let Some(end_date) = end_date {
            conditions.push("date <= $end_date");
            query_bindings.push(("end_date", end_date as Parameter));
        }

        let mut query = format_sstr!(
            "SELECT {select_str} FROM heartrate_statistics_summary {} {order_str}",
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
        query_dyn!(&query, ..query_bindings)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = Self::get_fitbit_statistics_query(
            "*",
            start_date.as_ref(),
            end_date.as_ref(),
            offset,
            limit,
            "ORDER BY date",
        )?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_total(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<usize, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }

        let query = Self::get_fitbit_statistics_query(
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
    /// Returns error if db query fails
    pub async fn upsert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        if Self::read_entry(self.date, pool).await?.is_some() {
            self.update_entry(pool).await
        } else {
            self.insert_entry(pool).await
        }
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn update_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                UPDATE heartrate_statistics_summary
                SET min_heartrate=$min_heartrate,max_heartrate=$max_heartrate,
                    mean_heartrate=$mean_heartrate,median_heartrate=$median_heartrate,
                    stdev_heartrate=$stdev_heartrate,number_of_entries=$number_of_entries
                WHERE date=$date
            "#,
            date = self.date,
            min_heartrate = self.min_heartrate,
            max_heartrate = self.max_heartrate,
            mean_heartrate = self.mean_heartrate,
            median_heartrate = self.median_heartrate,
            stdev_heartrate = self.stdev_heartrate,
            number_of_entries = self.number_of_entries,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                INSERT INTO heartrate_statistics_summary
                (date, min_heartrate, max_heartrate, mean_heartrate, median_heartrate,
                 stdev_heartrate, number_of_entries)
                VALUES
                ($date, $min_heartrate, $max_heartrate, $mean_heartrate, $median_heartrate,
                 $stdev_heartrate, $number_of_entries)
            "#,
            date = self.date,
            min_heartrate = self.min_heartrate,
            max_heartrate = self.max_heartrate,
            mean_heartrate = self.mean_heartrate,
            median_heartrate = self.median_heartrate,
            stdev_heartrate = self.stdev_heartrate,
            number_of_entries = self.number_of_entries,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }
}
