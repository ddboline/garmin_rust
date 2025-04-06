use futures::{stream::FuturesUnordered, TryStreamExt};
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::format_sstr;
use std::{collections::HashSet, convert::TryInto, fmt, sync::Arc};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use garmin_lib::{
    date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
    errors::GarminError as Error,
    garmin_config::GarminConfig,
};
use garmin_utils::pgpool::PgPool;

pub const GRAMS_PER_OUNCE: f64 = 28.349_523_125;
pub const LBS_PER_KG: f64 = 1_000.0 / (16.0 * GRAMS_PER_OUNCE);
pub const GRAMS_PER_POUND: f64 = GRAMS_PER_OUNCE * 16.0;

#[derive(Debug, Clone, Serialize, Deserialize, Copy, FromSqlRow, PartialEq)]
pub struct ScaleMeasurement {
    pub id: Uuid,
    pub datetime: DateTimeWrapper,
    pub mass: f64,
    pub fat_pct: f64,
    pub water_pct: f64,
    pub muscle_pct: f64,
    pub bone_pct: f64,
    pub connect_primary_key: Option<i64>,
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
    pub fn mass_in_grams(&self) -> f64 {
        self.mass * GRAMS_PER_POUND
    }

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
    pub fn from_fit_plus(
        weight_in_lbs: f64,
        body_fat_percent: f64,
        muscle_mass_lbs: f64,
        body_water_percent: f64,
        bone_mass_lbs: f64,
    ) -> Result<Self, Error> {
        if [
            weight_in_lbs,
            body_fat_percent,
            muscle_mass_lbs,
            body_water_percent,
            bone_mass_lbs,
        ]
        .iter()
        .any(|x| { *x < 0.0 } || { !x.is_finite() })
        {
            return Err(Error::StaticCustomError("Values cannot be negative"));
        }
        if weight_in_lbs > 1e3 {
            return Err(Error::StaticCustomError("Invalid Value"));
        }
        if muscle_mass_lbs + bone_mass_lbs > weight_in_lbs {
            return Err(Error::StaticCustomError(
                "Invalid inputs, muscle and bone masses must be less than total weight",
            ));
        }
        let id = Uuid::new_v4();
        let datetime = OffsetDateTime::now_utc().into();
        let muscle_pct = (muscle_mass_lbs / weight_in_lbs) * 100.0;
        let bone_pct = (bone_mass_lbs / weight_in_lbs) * 100.0;
        Ok(Self {
            id,
            datetime,
            mass: weight_in_lbs,
            fat_pct: body_fat_percent,
            water_pct: body_water_percent,
            muscle_pct,
            bone_pct,
            connect_primary_key: None,
        })
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
            return Err(Error::StaticCustomError("Bad message"));
        };

        let values = msg
            .split(sep)
            .map(|x| {
                let y: i32 = x.parse()?;
                if y < 0 {
                    return Err(Error::StaticCustomError("Bad message"));
                }
                Ok(f64::from(y) / 10.)
            })
            .take(5)
            .collect::<Result<SmallVec<[f64; 5]>, Error>>()?;

        if values.len() < 5 {
            return Err(Error::StaticCustomError("Bad message"));
        }

        Ok(Self {
            id: Uuid::new_v4(),
            datetime,
            mass: values[0],
            fat_pct: values[1],
            water_pct: values[2],
            muscle_pct: values[3],
            bone_pct: values[4],
            connect_primary_key: None,
        })
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_by_id(id: Uuid, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM scale_measurements WHERE id = $id", id = id);
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_by_connect_primary_key(
        key: i64,
        pool: &PgPool,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM scale_measurements WHERE connect_primary_key = $key LIMIT 1",
            key = key
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_by_datetime(dt: OffsetDateTime, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT * FROM scale_measurements WHERE datetime = $dt LIMIT 1",
            dt = dt
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn delete_from_db(self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "DELETE FROM scale_measurements WHERE id = $id",
            id = self.id
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn set_connect_primary_key(
        &mut self,
        primary_key: i64,
        pool: &PgPool,
    ) -> Result<(), Error> {
        self.connect_primary_key.replace(primary_key);

        let query = query!(
            "UPDATE scale_measurements SET connect_primary_key = $connect_primary_key WHERE id = \
             $id",
            id = self.id,
            connect_primary_key = self.connect_primary_key,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_into_db(&mut self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                INSERT INTO scale_measurements (
                    datetime, mass, fat_pct, water_pct, muscle_pct, bone_pct, connect_primary_key
                )
                VALUES ($datetime,$mass,$fat,$water,$muscle,$bone,$connect_primary_key)
            ",
            datetime = self.datetime,
            mass = self.mass,
            fat = self.fat_pct,
            water = self.water_pct,
            muscle = self.muscle_pct,
            bone = self.bone_pct,
            connect_primary_key = self.connect_primary_key,
        );

        let conn = pool.get().await?;
        query.execute(&conn).await?;

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
        let result = query.query_one(&conn).await?;
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

    fn get_scale_measurement_query<'a>(
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
            conditions.push("date(datetime) >= $start_date");
            query_bindings.push(("start_date", d as Parameter));
        }
        if let Some(d) = end_date {
            conditions.push("date(datetime) <= $end_date");
            query_bindings.push(("end_date", d as Parameter));
        }
        let mut query = format_sstr!(
            "SELECT {select_str} FROM scale_measurements {} {order_str}",
            if conditions.is_empty() {
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
        query_bindings.shrink_to_fit();
        debug!("query:\n{query}",);
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
    ) -> Result<Vec<Self>, Error> {
        let query = Self::get_scale_measurement_query(
            "*",
            start_date.as_ref(),
            end_date.as_ref(),
            offset,
            limit,
            "ORDER BY datetime",
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

        let query = Self::get_scale_measurement_query(
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
    pub async fn merge_updates<'a, T>(measurements: T, pool: &PgPool) -> Result<(), Error>
    where
        T: IntoIterator<Item = &'a mut Self>,
    {
        let mut measurement_set: HashSet<_> = Self::read_from_db(pool, None, None, None, None)
            .await?
            .into_par_iter()
            .map(|d| d.datetime)
            .collect();
        measurement_set.shrink_to_fit();
        let measurement_set = Arc::new(measurement_set);
        let futures: FuturesUnordered<_> = measurements
            .into_iter()
            .map(|meas| {
                let measurement_set = measurement_set.clone();
                async move {
                    if measurement_set.contains(&meas.datetime) {
                        debug!("measurement exists {meas:?}",);
                    } else {
                        meas.insert_into_db(pool).await?;
                        debug!("measurement inserted {meas:?}",);
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
    use log::debug;
    use time::{macros::datetime, OffsetDateTime};
    use uuid::Uuid;

    use garmin_lib::{errors::GarminError as Error, garmin_config::GarminConfig};
    use garmin_utils::pgpool::PgPool;

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
            connect_primary_key: None,
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
            connect_primary_key: None,
        };

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl)?;

        exp.insert_into_db(&pool).await?;

        let obs = ScaleMeasurement::get_by_id(exp.id, &pool).await?.unwrap();

        assert_eq!(exp, obs);

        let obs = ScaleMeasurement::get_by_datetime(exp.datetime.into(), &pool)
            .await?
            .unwrap();

        assert_eq!(exp, obs);

        let measurements = ScaleMeasurement::read_from_db(&pool, None, None, None, None).await?;
        assert!(measurements.len() > 0);
        let first = measurements[0];
        debug!("{:#?}", first);
        assert_eq!(first, exp);
        assert_eq!(first.datetime, first_date);

        exp.delete_from_db(&pool).await?;

        Ok(())
    }
}
