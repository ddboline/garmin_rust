use anyhow::{format_err, Error};
use chrono::{DateTime, Local, NaiveDate, Utc};
use log::debug;
use maplit::hashmap;
use postgres_query::{FromSqlRow, Parameter};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{self, Deserialize, Serialize};
use stack_string::StackString;
use std::{collections::HashMap, fmt};

use garmin_lib::{
    common::{garmin_templates::HBR, pgpool::PgPool},
    utils::iso_8601_datetime::convert_datetime_to_str,
};

#[derive(Debug, Clone, Serialize, Deserialize, Copy, FromSqlRow, PartialEq)]
pub struct ScaleMeasurement {
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
            "ScaleMeasurement(\ndatetime: {}\nmass: {} lbs\nfat: {}%\nwater: {}%\nmuscle: \
             {}%\nbone: {}%\n)",
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
        fn opt2res<T>(item: Option<Result<T, Error>>) -> Result<T, Error> {
            match item {
                Some(x) => x,
                None => Err(format_err!("Bad message")),
            }
        }

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

        let mut iter = msg.split(sep).map(|x| {
            let y: i32 = x.parse()?;
            if y < 0 {
                return Err(format_err!("Bad message"));
            }
            Ok(f64::from(y) / 10.)
        });

        Ok(Self {
            datetime,
            mass: opt2res(iter.next())?,
            fat_pct: opt2res(iter.next())?,
            water_pct: opt2res(iter.next())?,
            muscle_pct: opt2res(iter.next())?,
            bone_pct: opt2res(iter.next())?,
        })
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
            INSERT INTO scale_measurements (datetime, mass, fat_pct, water_pct, muscle_pct, \
             bone_pct)
            VALUES ($datetime,$mass,$fat,$water,$muscle,$bone)",
            datetime = self.datetime,
            mass = self.mass,
            fat = self.fat_pct,
            water = self.water_pct,
            muscle = self.muscle_pct,
            bone = self.bone_pct,
        );

        let conn = pool.get().await?;

        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
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
            conditions.push("date(datetime) >= $start_date".to_string());
            bindings.push(("start_date", d));
        }
        if let Some(d) = end_date {
            conditions.push("date(datetime) <= $end_date".to_string());
            bindings.push(("end_date", d));
        }
        let query = format!(
            "{} {} ORDER BY datetime",
            query,
            if conditions.is_empty() {
                "".to_string()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            }
        );
        let query_bindings: Vec<_> = bindings.iter().map(|(k, v)| (*k, v as Parameter)).collect();
        debug!("query:\n{}", query);
        let query = postgres_query::query_dyn!(&query, ..query_bindings)?;
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .par_iter()
            .map(|r| Self::from_row(r).map_err(Into::into))
            .collect()
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
                let key = meas.datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
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
                let key = meas.datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
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
                let key = meas.datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
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
                let key = meas.datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
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
                let key = meas.datetime.format("%Y-%m-%dT%H:%M:%S%z").to_string();
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
                format!(
                    r#"
                    <td>{}</td><td>{:3.1}</td><td>{:2.1}</td><td>{:2.1}</td>
                    <td>{:2.1}</td><td>{:2.1}</td>"#,
                    date, meas.mass, meas.fat_pct, meas.water_pct, meas.muscle_pct, meas.bone_pct,
                )
            })
            .collect();
        let entries = format!(
            r#"
            <table border=1>
            <thead>
            <th>Date</th><th>Weight</th><th>Fat %</th><th>Water %</th>
            <th>Muscle %</th><th>Bone %</th>
            </thead>
            <tbody>
            <tr>{}</tr>
            </tbody>
            </table>
            <br>{}{}"#,
            entries.join("</tr><tr>"),
            if offset >= 10 {
                format!(
                    r#"<button type="submit" onclick="scale_measurement_plots({});">Previous</button>"#,
                    offset - 10
                )
            } else {
                "".to_string()
            },
            format!(
                r#"<button type="submit" onclick="scale_measurement_plots({});">Next</button>"#,
                offset + 10
            ),
        );
        let graphs = graphs.join("\n");

        Ok(hashmap! {
            "INSERTOTHERTEXTHERE".into() => "".into(),
            "INSERTOTHERIMAGESHERE".into() => graphs.into(),
            "INSERTTEXTHERE".into() => entries.into(),
        })
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
            datetime: Utc::now(),
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
    #[ignore]
    async fn test_read_scale_measurement_from_db() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let measurements = ScaleMeasurement::read_from_db(&pool, None, None).await?;
        assert!(measurements.len() > 700);
        let first = measurements[0];
        println!("{:#?}", first);
        let first_date: DateTime<Utc> = "2016-02-24T04:00:00-05:00".parse()?;
        assert_eq!(
            first,
            ScaleMeasurement {
                datetime: first_date,
                mass: 174.8,
                fat_pct: 18.2,
                water_pct: 61.5,
                muscle_pct: 41.8,
                bone_pct: 4.6,
            }
        );
        assert_eq!(first.datetime, first_date);
        Ok(())
    }
}
