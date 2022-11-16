use anyhow::{format_err, Error};
use futures::{stream::FuturesUnordered, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{self, Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::format_sstr;
use std::{collections::HashSet, fmt, sync::Arc};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use garmin_lib::{
    common::{garmin_config::GarminConfig, pgpool::PgPool},
    utils::date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
};

#[derive(Debug, Clone, Serialize, Deserialize, Copy, FromSqlRow, PartialEq)]
pub struct ScaleMeasurement {
    pub id: Uuid,
    pub datetime: DateTimeWrapper,
    pub mass: f64,
    pub fat_pct: f64,
    pub water_pct: f64,
    pub muscle_pct: f64,
    pub bone_pct: f64,
}

impl fmt::Display for ScaleMeasurement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ScaleMeasurement(\nid: {}\ndatetime: {}\nmass: {} lbs\nfat: {}%\nwater: {}%\nmuscle: \
             {}%\nbone: {}%\n)",
            self.id,
            convert_datetime_to_str(self.datetime.into()),
            self.mass,
            self.fat_pct,
            self.water_pct,
            self.muscle_pct,
            self.bone_pct,
        )
    }
}

impl ScaleMeasurement {
    #[must_use]
    pub fn get_bmi(&self, config: &GarminConfig) -> f64 {
        // Mass in Kg
        let mass = self.mass * (1.0 / 2.204_623);
        // Height in cm
        let height = config.height * 0.0254;
        mass / (height * height)
    }

    /// # Errors
    /// Returns error parsing msg fails
    pub fn from_telegram_text(msg: &str) -> Result<Self, Error> {
        let datetime = DateTimeWrapper::now();
        let sep = if msg.contains(',') {
            ','
        } else if msg.contains(':') {
            ':'
        } else if msg.contains('=') {
            '='
        } else {
            return Err(format_err!("Bad message"));
        };

        let values = msg
            .split(sep)
            .map(|x| {
                let y: i32 = x.parse()?;
                if y < 0 {
                    return Err(format_err!("Bad message"));
                }
                Ok(f64::from(y) / 10.)
            })
            .take(5)
            .collect::<Result<SmallVec<[f64; 5]>, Error>>()?;

        if values.len() < 5 {
            return Err(format_err!("Bad message"));
        }

        Ok(Self {
            id: Uuid::new_v4(),
            datetime,
            mass: values[0],
            fat_pct: values[1],
            water_pct: values[2],
            muscle_pct: values[3],
            bone_pct: values[4],
        })
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_by_id(id: Uuid, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM scale_measurements WHERE id = $id", id = id);
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_by_datetime(dt: OffsetDateTime, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM scale_measurements WHERE datetime = $dt",
            dt = dt
        );
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn delete_from_db(self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "DELETE FROM scale_measurements WHERE id = $id",
            id = self.id
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await?;
        Ok(())
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_into_db(&mut self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                INSERT INTO scale_measurements (
                    datetime, mass, fat_pct, water_pct, muscle_pct, bone_pct
                )
                VALUES ($datetime,$mass,$fat,$water,$muscle,$bone)
            ",
            datetime = self.datetime,
            mass = self.mass,
            fat = self.fat_pct,
            water = self.water_pct,
            muscle = self.muscle_pct,
            bone = self.bone_pct,
        );

        let conn = pool.get().await?;

        conn.execute(query.sql(), query.parameters()).await?;

        let query = query!(
            "
                SELECT id
                FROM scale_measurements
                WHERE datetime = $datetime
                    AND mass = $mass
                    AND fat_pct = $fat
                    AND water_pct = $water
                    AND muscle_pct = $muscle
                    AND bone_pct = $bone
            ",
            datetime = self.datetime,
            mass = self.mass,
            fat = self.fat_pct,
            water = self.water_pct,
            muscle = self.muscle_pct,
            bone = self.bone_pct,
        );
        let result = conn.query_one(query.sql(), query.parameters()).await?;
        self.id = result.try_get("id")?;

        Ok(())
    }

    /// # Errors
    /// Returns error if reading files fails
    pub async fn read_latest_from_db(pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = "
            SELECT * FROM scale_measurements
            WHERE datetime = (
                SELECT max(datetime) FROM scale_measurements
            )";
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query, &[])
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<Vec<Self>, Error> {
        let query = "SELECT * FROM scale_measurements";
        let mut conditions = Vec::new();
        let mut bindings = Vec::new();
        if let Some(d) = start_date {
            conditions.push("date(datetime) >= $start_date");
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(datetime) <= $end_date");
            bindings.push(("end_date", d));
        }
        let query = format_sstr!(
            "{query} {c} ORDER BY datetime",
            c = if conditions.is_empty() {
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

    /// # Errors
    /// Returns error if db query fails
    pub async fn merge_updates<'a, T>(measurements: T, pool: &PgPool) -> Result<(), Error>
    where
        T: IntoIterator<Item = &'a mut Self>,
    {
        let measurement_set: HashSet<_> = ScaleMeasurement::read_from_db(pool, None, None)
            .await?
            .into_par_iter()
            .map(|d| d.datetime)
            .collect();
        let measurement_set = Arc::new(measurement_set);
        let futures: FuturesUnordered<_> = measurements
            .into_iter()
            .map(|meas| {
                let measurement_set = measurement_set.clone();
                async move {
                    if measurement_set.contains(&meas.datetime) {
                        debug!("measurement exists {:?}", meas);
                    } else {
                        meas.insert_into_db(pool).await?;
                        debug!("measurement inserted {:?}", meas);
                    }
                    Ok(())
                }
            })
            .collect();
        futures.try_collect().await
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use log::debug;
    use time::{macros::datetime, OffsetDateTime};
    use uuid::Uuid;

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::scale_measurement::ScaleMeasurement;

    #[test]
    fn test_from_telegram_text() -> Result<(), Error> {
        let msg = "1880=206=596=404=42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        let mut exp = ScaleMeasurement {
            id: obs.id,
            datetime: OffsetDateTime::now_utc().into(),
            mass: 188.0,
            fat_pct: 20.6,
            water_pct: 59.6,
            muscle_pct: 40.4,
            bone_pct: 4.2,
        };
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        let msg = "1880,206,596,404,42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        exp.id = obs.id;
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        let msg = "1880:206:596:404:42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        exp.id = obs.id;
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        Ok(())
    }

    #[tokio::test]
    async fn test_write_read_scale_measurement_from_db() -> Result<(), Error> {
        let first_date = datetime!(2010-01-01 04:00:00 -05:00).into();
        let mut exp = ScaleMeasurement {
            id: Uuid::new_v4(),
            datetime: first_date,
            mass: 188.0,
            fat_pct: 20.6,
            water_pct: 59.6,
            muscle_pct: 40.4,
            bone_pct: 4.2,
        };

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        exp.insert_into_db(&pool).await?;

        let obs = ScaleMeasurement::get_by_id(exp.id, &pool).await?.unwrap();

        assert_eq!(exp, obs);

        let obs = ScaleMeasurement::get_by_datetime(exp.datetime.into(), &pool)
            .await?
            .unwrap();

        assert_eq!(exp, obs);

        let measurements = ScaleMeasurement::read_from_db(&pool, None, None).await?;
        assert!(measurements.len() > 0);
        let first = measurements[0];
        debug!("{:#?}", first);
        assert_eq!(first, exp);
        assert_eq!(first.datetime, first_date);

        exp.delete_from_db(&pool).await?;

        Ok(())
    }
}
