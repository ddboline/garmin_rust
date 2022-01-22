use anyhow::{format_err, Error};
use chrono::{DateTime, Local, NaiveDate, Utc};
use futures::future::try_join_all;
use log::debug;
use maplit::hashmap;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{self, Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    fmt::Write,
    sync::Arc,
};

use garmin_lib::{
    common::{garmin_templates::HBR, pgpool::PgPool},
    utils::iso_8601_datetime::convert_datetime_to_str,
};

#[derive(Debug, Clone, Serialize, Deserialize, Copy, FromSqlRow, PartialEq)]
pub struct ScaleMeasurement {
    pub id: i32,
    pub datetime: DateTime<Utc>,
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
            convert_datetime_to_str(self.datetime),
            self.mass,
            self.fat_pct,
            self.water_pct,
            self.muscle_pct,
            self.bone_pct,
        )
    }
}

impl ScaleMeasurement {
    pub fn from_telegram_text(msg: &str) -> Result<Self, Error> {
        let datetime = Utc::now();
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
            id: -1,
            datetime,
            mass: values[0],
            fat_pct: values[1],
            water_pct: values[2],
            muscle_pct: values[3],
            bone_pct: values[4],
        })
    }

    pub async fn get_by_id(id: i32, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!("SELECT * FROM scale_measurements WHERE id = $id", id = id);
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    pub async fn get_by_datetime(dt: DateTime<Utc>, pool: &PgPool) -> Result<Option<Self>, Error> {
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

    pub async fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
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

    pub fn get_scale_measurement_plots(
        measurements: &[Self],
        offset: Option<usize>,
    ) -> Result<HashMap<StackString, StackString>, Error> {
        let offset = offset.unwrap_or(0);
        if measurements.is_empty() {
            return Ok(hashmap! {
                "INSERTOTHERIMAGESHERE".into() => "".into(),
                "INSERTTEXTHERE".into() => "".into(),
            });
        }
        let mut graphs = Vec::new();

        let mass: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let key = StackString::from_display(meas.datetime.format("%Y-%m-%dT%H:%M:%S%z"));
                (key, meas.mass)
            })
            .collect();

        let js_str = serde_json::to_string(&mass).unwrap_or_else(|_| "".to_string());
        let params = hashmap! {
            "EXAMPLETITLE" => "Weight",
            "XAXIS" => "Date",
            "YAXIS" => "Weight [lbs]",
            "DATA" => &js_str,
            "NAME" => "weight",
            "UNITS" => "lbs",
        };
        let plot: StackString = HBR.render("TIMESERIESTEMPLATE", &params)?.into();
        graphs.push(plot);

        let fat: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let key = StackString::from_display(meas.datetime.format("%Y-%m-%dT%H:%M:%S%z"));
                (key, meas.fat_pct)
            })
            .collect();

        let js_str = serde_json::to_string(&fat).unwrap_or_else(|_| "".to_string());
        let params = hashmap! {
            "EXAMPLETITLE" => "Fat %",
            "XAXIS" => "Date",
            "YAXIS" => "Fat %",
            "DATA" => &js_str,
            "NAME" => "fat",
            "UNITS" => "%",
        };
        let plot = HBR.render("TIMESERIESTEMPLATE", &params)?;
        graphs.push(plot.into());

        let water: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let key = StackString::from_display(meas.datetime.format("%Y-%m-%dT%H:%M:%S%z"));
                (key, meas.water_pct)
            })
            .collect();

        let js_str = serde_json::to_string(&water).unwrap_or_else(|_| "".to_string());
        let params = hashmap! {
            "EXAMPLETITLE" => "Water %",
            "XAXIS" => "Date",
            "YAXIS" => "Water %",
            "DATA" => &js_str,
            "NAME" => "water",
            "UNITS" => "%",
        };
        let plot = HBR.render("TIMESERIESTEMPLATE", &params)?;
        graphs.push(plot.into());

        let muscle: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let key = StackString::from_display(meas.datetime.format("%Y-%m-%dT%H:%M:%S%z"));
                (key, meas.muscle_pct)
            })
            .collect();

        let js_str = serde_json::to_string(&muscle).unwrap_or_else(|_| "".to_string());
        let params = hashmap! {
            "EXAMPLETITLE" => "Muscle %",
            "XAXIS" => "Date",
            "YAXIS" => "Muscle %",
            "DATA" => &js_str,
            "NAME" => "muscle",
            "UNITS" => "%",
        };
        let plot = HBR.render("TIMESERIESTEMPLATE", &params)?;
        graphs.push(plot.into());

        let bone: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let key = StackString::from_display(meas.datetime.format("%Y-%m-%dT%H:%M:%S%z"));
                (key, meas.bone_pct)
            })
            .collect();

        let js_str = serde_json::to_string(&bone).unwrap_or_else(|_| "".to_string());
        let params = hashmap! {
            "EXAMPLETITLE" => "Bone %",
            "XAXIS" => "Date",
            "YAXIS" => "Bone %",
            "DATA" => &js_str,
            "NAME" => "bone",
            "UNITS" => "%",
        };
        let plot = HBR.render("TIMESERIESTEMPLATE", &params)?;
        graphs.push(plot.into());

        let n = measurements.len();
        let entries: Vec<_> = measurements[(n - 10 - offset)..(n - offset)]
            .iter()
            .map(|meas| {
                let date = meas.datetime.with_timezone(&Local).date().naive_local();
                format_sstr!(
                    r#"
                    <td>{date}</td><td>{m:3.1}</td><td>{f:2.1}</td><td>{w:2.1}</td>
                    <td>{ms:2.1}</td><td>{b:2.1}</td>"#,
                    m = meas.mass,
                    f = meas.fat_pct,
                    w = meas.water_pct,
                    ms = meas.muscle_pct,
                    b = meas.bone_pct,
                )
            })
            .collect();
        let entries = format_sstr!(
            r#"
            <table border=1>
            <thead>
            <th>Date</th>
            <th><a href="https://www.fitbit.com/weight" target="_blank">Weight</a></th>
            <th>Fat %</th><th>Water %</th>
            <th>Muscle %</th><th>Bone %</th>
            </thead>
            <tbody>
            <tr>{}</tr>
            </tbody>
            </table>
            <br>{}{}"#,
            entries.join("</tr><tr>"),
            if offset >= 10 {
                format_sstr!(
                    r#"<button type="submit" onclick="scale_measurement_plots({});">Previous</button>"#,
                    offset - 10
                )
            } else {
                "".into()
            },
            format_sstr!(
                r#"<button type="submit" onclick="scale_measurement_plots({});">Next</button>"#,
                offset + 10
            ),
        );
        let graphs = graphs.join("\n");

        Ok(hashmap! {
            "INSERTOTHERIMAGESHERE".into() => "".into(),
            "INSERTTABLESHERE".into() => graphs.into(),
            "INSERTTEXTHERE".into() => entries,
        })
    }

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
        let futures = measurements.into_iter().map(|meas| {
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
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{DateTime, Utc};

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::scale_measurement::ScaleMeasurement;

    #[test]
    fn test_from_telegram_text() -> Result<(), Error> {
        let mut exp = ScaleMeasurement {
            id: -1,
            datetime: Utc::now().into(),
            mass: 188.0,
            fat_pct: 20.6,
            water_pct: 59.6,
            muscle_pct: 40.4,
            bone_pct: 4.2,
        };
        let msg = "1880=206=596=404=42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        let msg = "1880,206,596,404,42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        let msg = "1880:206:596:404:42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;
        exp.datetime = obs.datetime;
        assert_eq!(obs, exp);
        Ok(())
    }

    #[tokio::test]
    async fn test_write_read_scale_measurement_from_db() -> Result<(), Error> {
        let first_date: DateTime<Utc> = "2010-01-01T04:00:00-05:00".parse()?;
        let mut exp = ScaleMeasurement {
            id: -1,
            datetime: first_date.into(),
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
        println!("{:#?}", first);
        assert_eq!(first, exp);
        assert_eq!(first.datetime, first_date);

        exp.delete_from_db(&pool).await?;

        Ok(())
    }
}
