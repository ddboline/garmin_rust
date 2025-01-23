use anyhow::Error;
use futures::TryStreamExt;
use ndarray::{array, Array1};
use postgres_query::{query, FromSqlRow};
use rusfun::{curve_fit::Minimizer, func1d::Func1D};
use serde::Serialize;
use stack_string::StackString;
use std::collections::HashMap;
use time::{Date, OffsetDateTime};
use time_tz::{OffsetDateTimeExt, Tz};
use uuid::Uuid;

use garmin_lib::date_time_wrapper::DateTimeWrapper;
use garmin_models::garmin_summary::GarminSummary;
use garmin_utils::{
    garmin_util::{print_h_m_s, MARATHON_DISTANCE_M, METERS_PER_MILE},
    pgpool::PgPool,
};

use crate::{race_results::RaceResults, race_type::RaceType};

#[derive(Serialize)]
pub struct RacePoint {
    pub x: i32,
    pub y: f64,
    pub name: StackString,
    pub date: Date,
    pub label: StackString,
}

#[derive(PartialEq, Clone)]
pub struct RaceResultAnalysis {
    pub data: Vec<RaceResults>,
    pub summary_map: HashMap<Uuid, GarminSummary>,
    parameters: Array1<f64>,
    errors: Array1<f64>,
    pub race_type: RaceType,
}

fn power_law(p: &Array1<f64>, x: &Array1<f64>) -> Array1<f64> {
    assert_eq!(p.len(), 3);
    let mdist = f64::from(MARATHON_DISTANCE_M) / METERS_PER_MILE;
    x.map(|x| {
        if x <= &mdist {
            p[0] * (x / mdist).powf(p[1])
        } else {
            p[0] * (x / mdist).powf(p[2])
        }
    })
}

#[derive(Copy, Clone)]
pub enum ParamType {
    Nom,
    Pos,
    Neg,
}

pub struct PlotData {
    pub data: Vec<RacePoint>,
    pub other_data: Vec<RacePoint>,
    pub x_proj: Array1<f64>,
    pub y_proj: Array1<f64>,
    pub x_vals: Array1<f64>,
    pub y_nom: Array1<f64>,
    pub y_neg: Array1<f64>,
    pub y_pos: Array1<f64>,
    pub xticks: Vec<i32>,
    pub yticks: Vec<i32>,
    pub ymin: i32,
    pub ymax: i32,
}

impl RaceResultAnalysis {
    /// # Errors
    /// Return error if db queries fail
    pub async fn run_analysis(race_type: RaceType, pool: &PgPool) -> Result<Self, Error> {
        let mut data: Vec<_> = RaceResults::get_results_by_type(race_type, pool)
            .await?
            .try_collect()
            .await?;
        data.shrink_to_fit();
        let mut summary_map = RaceResults::get_summary_map(pool).await?;
        summary_map.shrink_to_fit();
        let mut agg_results =
            RaceResultAggregated::get_aggregated_race_results(race_type, pool).await?;
        agg_results.shrink_to_fit();

        let x_values: Array1<f64> = agg_results
            .iter()
            .map(|r| f64::from(r.race_distance) / METERS_PER_MILE)
            .collect();
        let y_values: Array1<f64> = agg_results
            .iter()
            .map(|r| (METERS_PER_MILE / 60.0) * r.race_pace_mean)
            .collect();
        let s_values: Array1<f64> = agg_results
            .iter()
            .map(|r| (METERS_PER_MILE / 60.0) * r.race_pace_stddev)
            .collect();
        let (p0, n) = agg_results
            .iter()
            .filter_map(|r| {
                if (r.race_distance - MARATHON_DISTANCE_M).abs() < 1000 {
                    Some(r.race_pace_mean)
                } else {
                    None
                }
            })
            .fold((0.0, 0), |(s, n), dur| (s + dur, n + 1));
        let params = array![p0 / f64::from(n), 1.0, 1.0];
        let flags = array![true, true, true];

        let model_function = Func1D::new(&params, &x_values, power_law);
        let mut minimizer = Minimizer::init(&model_function, &y_values, &s_values, &flags, 0.1);
        minimizer.minimize();
        Ok(Self {
            data,
            summary_map,
            parameters: minimizer.minimizer_parameters,
            errors: minimizer.parameter_errors,
            race_type,
        })
    }

    #[must_use]
    pub fn params(&self, param_type: ParamType) -> Array1<f64> {
        match param_type {
            ParamType::Nom => self.parameters.clone(),
            ParamType::Pos => self.parameters.clone() + &self.errors,
            ParamType::Neg => self.parameters.clone() - &self.errors,
        }
    }

    /// # Errors
    /// Return error if template rendering fails
    #[must_use]
    pub fn get_data(&self) -> PlotData {
        fn extract_points(result: &RaceResults, tz: &Tz) -> RacePoint {
            let distance = f64::from(result.race_distance) / METERS_PER_MILE;
            let duration = result.race_time / 60.0;
            let x = result.race_distance;
            let y = duration / distance;
            RacePoint {
                x,
                y,
                name: result.race_name.clone().unwrap_or_else(|| "".into()),
                date: result.race_date.map_or_else(
                    || OffsetDateTime::now_utc().to_timezone(tz).date(),
                    Into::into,
                ),
                label: print_h_m_s(result.race_time, true).unwrap_or_else(|_| "".into()),
            }
        }
        let local = DateTimeWrapper::local_tz();
        let xticks = vec![
            100,
            200,
            400,
            800,
            METERS_PER_MILE as i32,
            5000,
            10_000,
            MARATHON_DISTANCE_M / 2,
            MARATHON_DISTANCE_M,
            50_000,
            50 * METERS_PER_MILE as i32,
            100 * METERS_PER_MILE as i32,
            300 * METERS_PER_MILE as i32,
        ];

        let (ymin, ymax) = match self.race_type {
            RaceType::Personal => (5, 24),
            RaceType::WorldRecordMen => (2, 12),
            RaceType::WorldRecordWomen => (2, 16),
        };
        let mut yticks: Vec<i32> = (ymin..ymax).collect();
        yticks.shrink_to_fit();

        let (data, other_data): (Vec<_>, Vec<_>) =
            self.data.iter().partition(|result| result.race_flag);

        let mut data: Vec<_> = data.into_iter().map(|d| extract_points(d, local)).collect();
        data.shrink_to_fit();
        let mut other_data: Vec<_> = other_data
            .into_iter()
            .map(|d| extract_points(d, local))
            .collect();
        other_data.shrink_to_fit();

        let (xmin, xmax) = match self.race_type {
            RaceType::Personal => (1.0, 100.0),
            RaceType::WorldRecordMen | RaceType::WorldRecordWomen => (0.25, 300.0),
        };

        let x_vals = Array1::linspace(xmin, xmax, 200);
        let y_nom = power_law(&self.params(ParamType::Nom), &x_vals);
        let y_neg = power_law(&self.params(ParamType::Neg), &x_vals);
        let y_pos = power_law(&self.params(ParamType::Pos), &x_vals);

        let x_proj: Array1<f64> = xticks
            .iter()
            .map(|x| f64::from(*x) / METERS_PER_MILE)
            .collect();
        let y_proj = power_law(&self.params(ParamType::Nom), &x_proj);
        PlotData {
            data,
            other_data,
            x_proj,
            y_proj,
            x_vals,
            y_nom,
            y_neg,
            y_pos,
            xticks,
            yticks,
            ymin,
            ymax,
        }
    }
}

#[derive(Debug, Clone, FromSqlRow, PartialEq)]
pub struct RaceResultAggregated {
    pub race_distance: i32,
    pub race_pace_mean: f64,
    pub race_pace_stddev: f64,
    pub race_count: i64,
}

impl RaceResultAggregated {
    /// # Errors
    /// Return error if db query fails
    async fn get_aggregated_race_results(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = query!(
            "
            SELECT
                race_distance,
                AVG(race_time / race_distance) AS race_pace_mean,
                CASE
                    WHEN COUNT(*) = 1 THEN AVG(race_time / race_distance) * 0.10
                    ELSE STDDEV(race_time / race_distance) END AS race_pace_stddev,
                COUNT(*) AS race_count
            FROM race_results
            WHERE race_type = $race_type AND race_flag = 't'
            GROUP BY 1
            ORDER BY 1
        ",
            race_type = race_type
        );
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use log::debug;

    use garmin_lib::garmin_config::GarminConfig;
    use garmin_utils::pgpool::PgPool;

    use crate::{race_result_analysis::RaceResultAggregated, race_type::RaceType};

    #[tokio::test]
    #[ignore]
    async fn test_get_aggregated_race_results() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl)?;

        let results =
            RaceResultAggregated::get_aggregated_race_results(RaceType::Personal, &pool).await?;
        debug!("{:#?}", results);
        debug!("{}", results.len());
        assert!(results.len() >= 23);
        Ok(())
    }
}
