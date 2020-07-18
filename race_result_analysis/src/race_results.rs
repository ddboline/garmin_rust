use anyhow::Error;
use chrono::NaiveDate;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::garmin_util::print_h_m_s;

use crate::race_type::RaceType;

#[derive(Debug, Clone, Serialize, Deserialize, FromSqlRow, PartialEq)]
pub struct RaceResults {
    pub id: i32,
    pub race_type: RaceType,
    pub race_date: Option<NaiveDate>,
    pub race_name: Option<String>,
    pub race_distance: f64,
    pub race_time: f64,
    pub race_flag: bool,
}

impl Display for RaceResults {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RaceResults(\nid: {}\nrace_type: {}{}{}\nrace_distance: {} km\nrace_time: {}\nrace_flag: \
             {}\n)",
            self.id,
            self.race_type,
            if let Some(date) = self.race_date {format!("\nrace_date: {}", date)} else {"".to_string()},
            if let Some(name) = &self.race_name {format!("\nrace_name: {}", name)} else {"".to_string()},
            self.race_distance,
            print_h_m_s(self.race_time, true).unwrap_or_else(|_| "".into()),
            self.race_flag,
        )
    }
}

impl RaceResults {
    pub async fn get_results_by_type(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_type = $race_type",
            race_type = race_type
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn get_result_by_id(id: i32, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!("SELECT * FROM race_results WHERE id = $id", id = id);
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    pub async fn get_race_by_date(race_date: NaiveDate, pool: &PgPool) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_date = $race_date",
            race_date = race_date
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
            INSERT INTO race_results (race_type, race_date, race_name, race_distance, race_time,
                race_flag)
            VALUES ($race_type,$race_date,$race_name,$race_distance,$race_time,$race_flag)",
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "UPDATE race_results
            SET race_type=$race_type,race_date=$race_date,race_name=$race_name,
                race_distance=$race_distance,race_time=$race_time,race_flag=$race_flag
            WHERE id=$id",
            id = self.id,
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn upsert_db(&self, pool: &PgPool) -> Result<(), Error> {
        if Self::get_result_by_id(self.id, pool).await?.is_some() {
            self.update_db(pool).await?;
        } else {
            self.insert_into_db(pool).await?;
        }
        Ok(())
    }

    pub async fn delete_from_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!("DELETE FROM race_results WHERE id = $id", id=self.id);
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::NaiveDate;

    use garmin_lib::common::garmin_config::GarminConfig;
    use garmin_lib::common::pgpool::PgPool;

    use crate::race_results::RaceResults;
    use crate::race_type::RaceType;

    fn get_test_race_result() -> RaceResults {
        RaceResults {
            id: 0,
            race_type: RaceType::Personal,
            race_date: Some(NaiveDate::from_ymd(2020, 1, 27)),
            race_name: Some("A Test Race".to_string()),
            race_distance: 5.0,
            race_time: 1563.0,
            race_flag: false,
        }
    }

    #[test]
    fn test_race_results_display() -> Result<(), Error> {
        let result = get_test_race_result();
        let result = result.to_string();
        assert!(result.contains("race_time: 00:26:03"));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_insert_delete_entry() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let result = get_test_race_result();
        result.insert_into_db(&pool).await?;

        let db_result = RaceResults::get_race_by_date(NaiveDate::from_ymd(2020, 1, 27), &pool).await?;
        println!("{:?}", db_result);
        assert_eq!(db_result.len(), 1);
        for r in db_result {
            r.delete_from_db(&pool).await?;
        }
        let db_result = RaceResults::get_race_by_date(NaiveDate::from_ymd(2020, 1, 27), &pool).await?;
        assert_eq!(db_result.len(), 0);
        Ok(())
    }
}
