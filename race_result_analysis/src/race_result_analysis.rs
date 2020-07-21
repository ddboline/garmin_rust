use anyhow::Error;
use ndarray::{array, Array1};
use postgres_query::FromSqlRow;
use rusfun::curve_fit::Minimizer;
use rusfun::func1d::Func1D;

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::garmin_util::MARATHON_DISTANCE_M;

use crate::race_type::RaceType;

pub struct RaceResultAnalysis {}

fn power_law(p: &Array1<f64>, x: &Array1<f64>) -> Array1<f64> {
    assert_eq!(p.len(), 4);
    x.map(|x| {
        if x <= &p[0] {
            p[1] * (x / p[0]).powf(p[2])
        } else {
            p[1] * (x / p[0]).powf(p[3])
        }
    })
}

impl RaceResultAnalysis {
    pub async fn run_analysis(race_type: RaceType, pool: &PgPool) -> Result<(), Error> {
        let agg_results =
            RaceResultAggregated::get_aggregated_race_results(race_type, pool).await?;

        let x_values: Array1<f64> = agg_results.iter().map(|r| r.race_distance as f64).collect();
        let y_values: Array1<f64> = agg_results.iter().map(|r| r.race_duration_mean).collect();
        let s_values: Array1<f64> = agg_results.iter().map(|r| r.race_duration_stddev).collect();
        let (p0, n) = agg_results
            .iter()
            .filter_map(|r| {
                if (r.race_distance - MARATHON_DISTANCE_M).abs() < 1000 {
                    Some(r.race_duration_mean)
                } else {
                    None
                }
            })
            .fold((0.0, 0), |(s, n), dur| (s + dur, n + 1));
        let params = array![MARATHON_DISTANCE_M as f64, p0 / n as f64, 1.0, 1.0];
        let flags = array![true, true, true, true];
        let model_function = Func1D::new(&params, &x_values, power_law);
        let mut minimizer = Minimizer::init(&model_function, &y_values, &s_values, &flags, 0.01);
        minimizer.minimize();
        minimizer.report();
        Ok(())
    }
}

#[derive(Debug, Clone, FromSqlRow, PartialEq)]
pub struct RaceResultAggregated {
    pub race_distance: i32,
    pub race_duration_mean: f64,
    pub race_duration_stddev: f64,
    pub race_count: i64,
}

impl RaceResultAggregated {
    pub async fn get_aggregated_race_results(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "
            SELECT
                race_distance,
                AVG(race_time) AS race_duration_mean,
                CASE
                    WHEN COUNT(*) = 1 THEN AVG(race_time) * 0.05
                    ELSE STDDEV(race_time) END AS race_duration_stddev,
                COUNT(*) AS race_count
            FROM race_results
            WHERE race_type = $race_type AND race_flag = 't'
            GROUP BY 1
            ORDER BY 1
        ",
            race_type = race_type
        );
        pool.get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Datelike, NaiveDate, Utc};
    use std::collections::HashMap;

    use stack_string::StackString;

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::race_result_analysis::{RaceResultAggregated, RaceResultAnalysis};
    use crate::race_type::RaceType;

    #[tokio::test]
    #[ignore]
    async fn test_get_aggregated_race_results() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        let results =
            RaceResultAggregated::get_aggregated_race_results(RaceType::Personal, &pool).await?;
        println!("{:#?}", results);
        println!("{}", results.len());
        assert_eq!(results.len(), 23);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_run_analysis() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        RaceResultAnalysis::run_analysis(RaceType::Personal, &pool).await?;
        RaceResultAnalysis::run_analysis(RaceType::WorldRecordMen, &pool).await?;
        RaceResultAnalysis::run_analysis(RaceType::WorldRecordWomen, &pool).await?;
        assert!(false);
        Ok(())
    }
}
