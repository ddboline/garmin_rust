use chrono::offset::TimeZone;
use chrono::{DateTime, NaiveDate, Utc};
use failure::{err_msg, Error};
use google_sheets4::RowData;
use log::debug;
use serde::{self, Deserialize, Serialize};
use std::fmt;

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::iso_8601_datetime::convert_datetime_to_str;
use garmin_lib::utils::row_index_trait::RowIndexTrait;

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
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
    pub fn from_row_data(row_data: &RowData) -> Result<Self, Error> {
        let values = row_data
            .values
            .as_ref()
            .ok_or_else(|| err_msg("No values"))?;
        let values: Vec<_> = values
            .iter()
            .filter_map(|x| x.formatted_value.as_ref().map(|s| s.as_str()))
            .collect();
        if values.len() > 5 {
            let datetime = Utc
                .datetime_from_str(&values[0], "%_m/%e/%Y %k:%M:%S")
                .or_else(|_| {
                    DateTime::parse_from_rfc3339(&values[0]).map(|d| d.with_timezone(&Utc))
                })
                .or_else(|_| {
                    DateTime::parse_from_rfc3339(&values[0].replace(" ", "T"))
                        .map(|d| d.with_timezone(&Utc))
                })
                .or_else(|e| {
                    debug!("{} {}", values[0], e);
                    Err(e)
                })?;
            let mass: f64 = values[1].parse()?;
            let fat_pct: f64 = values[2].parse()?;
            let water_pct: f64 = values[3].parse()?;
            let muscle_pct: f64 = values[4].parse()?;
            let bone_pct: f64 = values[5].parse()?;
            Ok(Self {
                datetime,
                mass,
                fat_pct,
                water_pct,
                muscle_pct,
                bone_pct,
            })
        } else {
            Err(err_msg("Too few entries"))
        }
    }

    pub fn from_telegram_text(msg: &str) -> Result<Self, Error> {
        let datetime = Utc::now();
        let items: Result<Vec<f64>, Error> = if msg.contains(',') {
            msg.split(',')
        } else if msg.contains(':') {
            msg.split(':')
        } else if msg.contains('=') {
            msg.split('=')
        } else {
            return Err(err_msg("Bad message"));
        }
        .map(|x| {
            let y: i32 = x.parse()?;
            Ok(f64::from(y) / 10.)
        })
        .collect();

        let items = items?;

        if items.len() < 5 {
            return Err(err_msg("Bad message"));
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
        let query = "
            INSERT INTO scale_measurements (datetime, mass, fat_pct, water_pct, muscle_pct, bone_pct)
            VALUES ($1,$2,$3,$4,$5,$6)";
        let conn = pool.get()?;
        conn.execute(
            query,
            &[
                &self.datetime,
                &self.mass,
                &self.fat_pct,
                &self.water_pct,
                &self.muscle_pct,
                &self.bone_pct,
            ],
        )
        .map(|_| ())
        .map_err(err_msg)
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
        if let Some(d) = start_date {
            conditions.push(format!("datetime >= '{}'", d));
        }
        if let Some(d) = end_date {
            conditions.push(format!("datetime <= '{}'", d));
        }
        let query = format!(
            "{} {} ORDER BY datetime",
            query,
            if conditions.is_empty() {
                "".to_string()
            } else {
                conditions.join(" AND ")
            }
        );
        debug!("query:\n{}", query);
        let conn = pool.get()?;
        conn.query(&query, &[])?
            .iter()
            .map(|row| {
                let datetime: DateTime<Utc> = row.get_idx(0)?;
                let mass: f64 = row.get_idx(1)?;
                let fat_pct: f64 = row.get_idx(2)?;
                let water_pct: f64 = row.get_idx(3)?;
                let muscle_pct: f64 = row.get_idx(4)?;
                let bone_pct: f64 = row.get_idx(5)?;
                Ok(Self {
                    datetime,
                    mass,
                    fat_pct,
                    water_pct,
                    muscle_pct,
                    bone_pct,
                })
            })
            .collect()
    }
}
