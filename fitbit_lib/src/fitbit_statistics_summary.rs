use anyhow::Error;
use chrono::{DateTime, NaiveDate, Utc};
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use statistical::{mean, median, standard_deviation};

use garmin_lib::common::pgpool::PgPool;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, FromSqlRow)]
pub struct FitbitStatisticsSummary {
    pub date: NaiveDate,
    pub min_heartrate: f64,
    pub max_heartrate: f64,
    pub mean_heartrate: f64,
    pub median_heartrate: f64,
    pub stdev_heartrate: f64,
    pub number_of_entries: i32,
}

impl FitbitStatisticsSummary {
    pub fn from_heartrate_values(heartrate_values: &[(DateTime<Utc>, i32)]) -> Option<Self> {
        let date = heartrate_values[heartrate_values.len() / 2]
            .0
            .naive_local()
            .date();
        let min_heartrate = heartrate_values.iter().map(|(_, v)| *v).min()? as f64;
        let max_heartrate = heartrate_values.iter().map(|(_, v)| *v).max()? as f64;
        let values: Vec<_> = heartrate_values.iter().map(|(_, v)| *v as f64).collect();
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

    pub async fn read_entry(date: NaiveDate, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
            SELECT * FROM heartrate_statistics_summary WHERE date = $date
        "#,
            date = date
        );
        let conn = pool.get().await?;
        if let Some(row) = conn.query_opt(query.sql(), query.parameters()).await? {
            let val = Self::from_row(&row)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    pub async fn upsert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        if Self::read_entry(self.date, pool).await?.is_some() {
            self.update_entry(pool).await
        } else {
            self.insert_entry(pool).await
        }
    }

    pub async fn update_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
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
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn insert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO heartrate_statistics_summary
                (date, min_heartrate, max_heartrate, mean_heartrate, median_heartrate, stdev_heartrate, number_of_entries)
                VALUES
                ($date, $min_heartrate, $max_heartrate, $mean_heartrate, $median_heartrate, $stdev_heartrate, $number_of_entries)
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
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}
