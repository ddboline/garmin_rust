use anyhow::Error;
use ndarray::{array, Array1};
use postgres_query::FromSqlRow;
use rusfun::{curve_fit::Minimizer, func1d::Func1D};
use stack_string::StackString;
use std::collections::HashMap;

use garmin_lib::{
    common::pgpool::PgPool,
    reports::garmin_templates::{PLOT_TEMPLATE, PLOT_TEMPLATE_DEMO, SCATTERPLOTWITHLINES},
    utils::garmin_util::{MARATHON_DISTANCE_M, METERS_PER_MILE},
};

use crate::{race_results::RaceResults, race_type::RaceType};

pub struct RaceResultAnalysis {
    data: Vec<RaceResults>,
    parameters: Array1<f64>,
    errors: Array1<f64>,
    race_type: RaceType,
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

impl RaceResultAnalysis {
    pub async fn run_analysis(race_type: RaceType, pool: &PgPool) -> Result<Self, Error> {
        let data = RaceResults::get_results_by_type(race_type, pool).await?;
        let agg_results =
            RaceResultAggregated::get_aggregated_race_results(race_type, pool).await?;

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
            parameters: minimizer.minimizer_parameters,
            errors: minimizer.parameter_errors,
            race_type,
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
        fn extract_points(result: &RaceResults) -> (i32, f64) {
            let distance = f64::from(result.race_distance) / METERS_PER_MILE;
            let duration = result.race_time / 60.0;
            let x = result.race_distance;
            let y = duration / distance;
            (x, y)
        }

        let xticks: Vec<_> = [
            100,
            200,
            400,
            800,
            METERS_PER_MILE as i32,
            5000,
            10_000,
            MARATHON_DISTANCE_M / 2,
            MARATHON_DISTANCE_M,
            50 * METERS_PER_MILE as i32,
            100 * METERS_PER_MILE as i32,
            300 * METERS_PER_MILE as i32,
        ]
        .iter()
        .collect();
        let xlabels = [
            "100m", "", "", "800m", "1mi", "5k", "10k", "", "Marathon", "", "100mi", "300mi",
        ];
        let xmap: HashMap<_, _> = xticks.iter().zip(xlabels.iter()).collect();

        let (ymin, ymax) = match self.race_type {
            RaceType::Personal => (5, 24),
            RaceType::WorldRecordMen => (2, 12),
            RaceType::WorldRecordWomen => (2, 16),
        };
        let yticks: Vec<_> = (ymin..ymax).collect();

        let (data, other_data): (Vec<_>, Vec<_>) =
            self.data.iter().partition(|result| result.race_flag);

        let data: Vec<_> = data.into_iter().map(extract_points).collect();
        let other_data: Vec<_> = other_data.into_iter().map(extract_points).collect();

        let (xmin, xmax) = match self.race_type {
            RaceType::Personal => (1.0, 100.0),
            RaceType::WorldRecordMen | RaceType::WorldRecordWomen => (0.25, 300.0),
        };

        let x_vals = Array1::linspace(xmin, xmax, 200);
        let y_nom = power_law(&self.params(ParamType::Nom), &x_vals);
        let y_neg = power_law(&self.params(ParamType::Neg), &x_vals);
        let y_pos = power_law(&self.params(ParamType::Pos), &x_vals);

        let x_vals: Vec<f64> = x_vals.map(|x| x * METERS_PER_MILE).to_vec();
        let y_nom: Vec<(f64, f64)> = y_nom
            .iter()
            .zip(x_vals.iter())
            .map(|(y, x)| (*x, *y))
            .collect();
        let y_neg: Vec<(f64, f64)> = y_neg
            .iter()
            .zip(x_vals.iter())
            .map(|(y, x)| (*x, *y))
            .collect();
        let y_pos: Vec<(f64, f64)> = y_pos
            .iter()
            .zip(x_vals.iter())
            .map(|(y, x)| (*x, *y))
            .collect();

        let data = serde_json::to_string(&data).unwrap_or_else(|_| "".to_string());
        let other_data = serde_json::to_string(&other_data).unwrap_or_else(|_| "".to_string());
        let xticks = serde_json::to_string(&xticks).unwrap_or_else(|_| "".to_string());
        let xmap = serde_json::to_string(&xmap).unwrap_or_else(|_| "".to_string());
        let yticks = serde_json::to_string(&yticks).unwrap_or_else(|_| "".to_string());
        let fitdata = serde_json::to_string(&y_nom).unwrap_or_else(|_| "".to_string());
        let negdata = serde_json::to_string(&y_neg).unwrap_or_else(|_| "".to_string());
        let posdata = serde_json::to_string(&y_pos).unwrap_or_else(|_| "".to_string());
        let plots = SCATTERPLOTWITHLINES
            .replace("FITDATA", &fitdata)
            .replace("NEGDATA", &negdata)
            .replace("POSDATA", &posdata)
            .replace("OTHERDATA", &other_data)
            .replace("DATA", &data)
            .replace(
                "EXAMPLETITLE",
                match self.race_type {
                    RaceType::Personal => "Race Results",
                    RaceType::WorldRecordMen => "Men's World Record",
                    RaceType::WorldRecordWomen => "Women's World Record",
                },
            )
            .replace("XAXIS", "Distance")
            .replace("YAXIS", "Pace (min/mi)")
            .replace("XTICKS", &xticks)
            .replace("XMAP", &xmap)
            .replace("YTICKS", &yticks)
            .replace("YMIN", &ymin.to_string())
            .replace("YMAX", &ymax.to_string());
        let plots = format!("<script>\n{}\n</script>", plots);
        let buttons = [
            r#"<button type="submit" onclick="race_result_plot_personal();">Personal</button>"#,
            r#"<button type="submit" onclick="race_result_plot_world_record_men();">Mens World Records</button>"#,
            r#"<button type="submit" onclick="race_result_plot_world_record_women();">Womens World Records</button>"#,
        ];
        let template = if is_demo {
            PLOT_TEMPLATE_DEMO
        } else {
            PLOT_TEMPLATE
        };
        let body = template
            .replace("INSERTOTHERIMAGESHERE", &plots)
            .replace("INSERTTEXTHERE", &buttons.join(""))
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

    use garmin_lib::common::{garmin_config::GarminConfig, pgpool::PgPool};

    use crate::{
        race_result_analysis::{RaceResultAggregated, RaceResultAnalysis},
        race_type::RaceType,
    };

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
        let result = personal.create_plot(false)?;
        assert!(result.len() > 0);
        Ok(())
    }
}