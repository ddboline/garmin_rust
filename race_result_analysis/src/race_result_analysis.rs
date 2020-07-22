use anyhow::Error;
use chrono::{Duration, Local};
use ndarray::{array, Array1};
use postgres_query::FromSqlRow;
use rusfun::curve_fit::Minimizer;
use rusfun::func1d::Func1D;
use stack_string::StackString;
use std::path::Path;

use garmin_lib::common::pgpool::PgPool;
use garmin_lib::reports::garmin_templates::{
    PLOT_TEMPLATE, PLOT_TEMPLATE_DEMO, SCATTERPLOTWITHLINES,
};
use garmin_lib::utils::garmin_util::{MARATHON_DISTANCE_M, METERS_PER_MILE};

use crate::race_results::RaceResults;
use crate::race_type::RaceType;

pub struct RaceResultAnalysis {
    data: Vec<RaceResults>,
    parameters: Array1<f64>,
    errors: Array1<f64>,
}

fn power_law(p: &Array1<f64>, x: &Array1<f64>) -> Array1<f64> {
    assert_eq!(p.len(), 3);
    let mdist = MARATHON_DISTANCE_M as f64 / METERS_PER_MILE;
    x.map(|x| {
        if x <= &mdist {
            p[0] * (x / mdist).powf(p[1])
        } else {
            p[0] * (x / mdist).powf(p[2])
        }
    })
}

pub enum ParamType {
    Nom,
    Pos,
    Neg,
}

impl RaceResultAnalysis {
    pub async fn run_analysis(race_type: RaceType, pool: &PgPool) -> Result<Self, Error> {
        let data = RaceResults::get_results_by_type(race_type, pool).await?;
        let agg_results =
            RaceResultAggregated::get_aggregated_race_results(race_type, pool).await?;

        let x_values: Array1<f64> = agg_results
            .iter()
            .map(|r| r.race_distance as f64 / METERS_PER_MILE)
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
        let params = array![p0 / n as f64, 1.0, 1.0];
        let flags = array![true, true, true];
        let model_function = Func1D::new(&params, &x_values, power_law);
        let mut minimizer = Minimizer::init(&model_function, &y_values, &s_values, &flags, 0.1);
        minimizer.minimize();
        minimizer.report();
        Ok(Self {
            data,
            parameters: minimizer.minimizer_parameters,
            errors: minimizer.parameter_errors,
        })
    }

    pub fn params(&self, param_type: ParamType) -> Array1<f64> {
        match param_type {
            ParamType::Nom => self.parameters.clone(),
            ParamType::Pos => (self.parameters.clone() + &self.errors),
            ParamType::Neg => (self.parameters.clone() - &self.errors),
        }
    }

    pub fn create_plot(&self, is_demo: bool) -> Result<StackString, Error> {
        let data: Vec<(f64, f64)> = self
            .data
            .iter()
            .map(|result| {
                let distance = result.race_distance as f64 / METERS_PER_MILE;
                let duration = result.race_time / 60.0;
                let x = distance;
                let y = duration / distance;
                (x, y)
            })
            .collect();

        let js_str = serde_json::to_string(&data).unwrap_or_else(|_| "".to_string());
        let plots = SCATTERPLOTWITHLINES
            .replace("DATA", &js_str)
            .replace("EXAMPLETITLE", "Heart Rate")
            .replace("XAXIS", "Date")
            .replace("YAXIS", "Heart Rate");
        let plots = format!("<script>\n{}\n</script>", plots);
        let buttons: Vec<_> = (0..10)
            .map(|i| {
                let date = Local::today().naive_local() - Duration::days(i);
                format!(
                    "{}{}{}<br>",
                    format!(
                        r#"
                        <button type="submit" id="ID"
                         onclick="heartrate_plot_date('{date}','{date}');"">Plot {date}</button>"#,
                        date = date
                    ),
                    if is_demo {
                        "".to_string()
                    } else {
                        format!(
                            r#"
                        <button type="submit" id="ID"
                         onclick="heartrate_sync('{date}');">Sync {date}</button>
                        "#,
                            date = date
                        )
                    },
                    if is_demo {
                        "".to_string()
                    } else {
                        format!(
                            r#"
                        <button type="submit" id="ID"
                         onclick="connect_hr_sync('{date}');">Sync Garmin {date}</button>
                        "#,
                            date = date
                        )
                    },
                )
            })
            .collect();
        let template = if is_demo {
            PLOT_TEMPLATE_DEMO
        } else {
            PLOT_TEMPLATE
        };
        let body = template
            .replace("INSERTOTHERIMAGESHERE", &plots)
            .replace("INSERTTEXTHERE", "")
            .into();
        Ok(body)
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
    pub async fn get_aggregated_race_results(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "
            SELECT
                race_distance,
                AVG(race_distance / race_time) AS race_pace_mean,
                CASE
                    WHEN COUNT(*) = 1 THEN AVG(race_distance / race_time) * 0.10
                    ELSE STDDEV(race_distance / race_time) END AS race_pace_stddev,
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

        let personal = RaceResultAnalysis::run_analysis(RaceType::Personal, &pool).await?;
        personal.create_plot(false)?;
        // let _ = RaceResultAnalysis::run_analysis(RaceType::WorldRecordMen, &pool).await?;
        // let _ = RaceResultAnalysis::run_analysis(RaceType::WorldRecordWomen, &pool).await?;
        assert!(false);
        Ok(())
    }
}
