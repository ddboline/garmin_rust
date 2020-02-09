use anyhow::{format_err, Error};
use chrono::{DateTime, Local, NaiveDate, Utc};
use log::debug;
use postgres_query::{FromSqlRow, Parameter};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{self, Deserialize, Serialize};
use std::fmt;

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::reports::garmin_templates::PLOT_TEMPLATE;
use garmin_lib::utils::iso_8601_datetime::convert_datetime_to_str;
use garmin_lib::utils::plot_graph::generate_d3_plot;
use garmin_lib::utils::plot_opts::PlotOpts;

#[derive(Debug, Clone, Serialize, Deserialize, Copy, FromSqlRow)]
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
            "ScaleMeasurement(\ndatetime: {}\nmass: {} lbs\nfat: {}%\nwater: {}%\nmuscle: {}%\nbone: {}%\n)",
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
        let items: Result<Vec<f64>, Error> = if msg.contains(',') {
            msg.split(',')
        } else if msg.contains(':') {
            msg.split(':')
        } else if msg.contains('=') {
            msg.split('=')
        } else {
            return Err(format_err!("Bad message"));
        }
        .map(|x| {
            let y: i32 = x.parse()?;
            Ok(f64::from(y) / 10.)
        })
        .collect();

        let items = items?;

        if items.len() < 5 {
            return Err(format_err!("Bad message"));
        }
        Ok(Self {
            datetime,
            mass: items[0],
            fat_pct: items[1],
            water_pct: items[2],
            muscle_pct: items[3],
            bone_pct: items[4],
        })
    }

    pub fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!("
            INSERT INTO scale_measurements (datetime, mass, fat_pct, water_pct, muscle_pct, bone_pct)
            VALUES ($datetime,$mass,$fat,$water,$muscle,$bone)",
            datetime = self.datetime,
            mass = self.mass,
            fat = self.fat_pct,
            water = self.water_pct,
            muscle = self.muscle_pct,
            bone = self.bone_pct,
        );

        let mut conn = pool.get()?;

        conn.execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn read_from_db(
        pool: &PgPool,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<Self>, Error> {
        let query = "
            SELECT datetime, mass, fat_pct, water_pct, muscle_pct, bone_pct
            FROM scale_measurements
        ";
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
        let mut conn = pool.get()?;
        conn.query(query.sql(), query.parameters())?
            .par_iter()
            .map(|r| Self::from_row(r).map_err(Into::into))
            .collect()
    }

    pub fn get_scale_measurement_plots(measurements: &[Self]) -> Result<String, Error> {
        if measurements.is_empty() {
            let body = PLOT_TEMPLATE
                .replace("INSERTOTHERIMAGESHERE", "")
                .replace("INSERTTEXTHERE", "");
            return Ok(body);
        }
        let mut graphs = Vec::new();
        let start_datetime = measurements[0].datetime;

        let mass: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let days = (meas.datetime - start_datetime).num_days();
                (days as f64, meas.mass)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("weight")
            .with_title("Weight")
            .with_data(&mass)
            .with_labels("days", "Weight [lbs]");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let fat: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let days = (meas.datetime - start_datetime).num_days();
                (days as f64, meas.fat_pct)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("fat")
            .with_title("Fat %")
            .with_data(&fat)
            .with_labels("days", "Fat %");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let water: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let days = (meas.datetime - start_datetime).num_days();
                (days as f64, meas.water_pct)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("water")
            .with_title("Water %")
            .with_data(&water)
            .with_labels("days", "Water %");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let muscle: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let days = (meas.datetime - start_datetime).num_days();
                (days as f64, meas.muscle_pct)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("muscle")
            .with_title("Muscle %")
            .with_data(&muscle)
            .with_labels("days", "Muscle %");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let bone: Vec<_> = measurements
            .iter()
            .map(|meas| {
                let days = (meas.datetime - start_datetime).num_days();
                (days as f64, meas.bone_pct)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("bone")
            .with_title("Bone %")
            .with_data(&bone)
            .with_labels("days", "Bone %");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let n = measurements.len();
        let entries: Vec<_> = measurements[n - 10..n]
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
            </table>"#,
            entries.join("</tr><tr>")
        );

        let body = PLOT_TEMPLATE
            .replace("INSERTOTHERIMAGESHERE", &graphs.join("\n"))
            .replace("INSERTTEXTHERE", &entries);
        Ok(body)
    }
}
