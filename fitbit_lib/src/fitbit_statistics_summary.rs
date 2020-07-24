use anyhow::Error;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use statistical::{mean, median, standard_deviation};

use garmin_lib::{
    common::pgpool::PgPool,
    reports::garmin_templates::{PLOT_TEMPLATE, PLOT_TEMPLATE_DEMO},
    utils::{plot_graph::generate_d3_plot, plot_opts::PlotOpts},
};

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, FromSqlRow)]
pub struct FitbitStatisticsSummary {
    pub date: NaiveDate,
    pub min_heartrate: f64,
    pub max_heartrate: f64,
    pub mean_heartrate: f64,
    pub median_heartrate: f64,
    pub stdev_heartrate: f64,
    pub number_of_entries: i32,
}

impl FitbitStatisticsSummary {
    pub fn from_heartrate_values(heartrate_values: &[(DateTime<Utc>, i32)]) -> Option<Self> {
        let date = heartrate_values[heartrate_values.len() / 2]
            .0
            .naive_local()
            .date();
        let min_heartrate = f64::from(heartrate_values.iter().map(|(_, v)| *v).min()?);
        let max_heartrate = f64::from(heartrate_values.iter().map(|(_, v)| *v).max()?);
        let values: Vec<_> = heartrate_values
            .iter()
            .map(|(_, v)| f64::from(*v))
            .collect();
        let mean_heartrate = mean(&values);
        let median_heartrate = median(&values);
        let stdev_heartrate = standard_deviation(&values, Some(mean_heartrate));
        Some(Self {
            date,
            min_heartrate,
            max_heartrate,
            mean_heartrate,
            median_heartrate,
            stdev_heartrate,
            number_of_entries: values.len() as i32,
        })
    }

    pub async fn read_entry(date: NaiveDate, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
            SELECT * FROM heartrate_statistics_summary WHERE date = $date
        "#,
            date = date
        );
        let conn = pool.get().await?;
        if let Some(row) = conn.query_opt(query.sql(), query.parameters()).await? {
            let val = Self::from_row(&row)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    pub async fn read_from_db(
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let start_date =
            start_date.unwrap_or_else(|| (Utc::now() - Duration::days(365)).naive_local().date());
        let end_date = end_date.unwrap_or_else(|| Utc::now().naive_local().date());

        let query = postgres_query::query!(
            r#"
            SELECT * FROM heartrate_statistics_summary
            WHERE date >= $start_date AND date <= $end_date
            ORDER BY date
        "#,
            start_date = start_date,
            end_date = end_date
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn upsert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        if Self::read_entry(self.date, pool).await?.is_some() {
            self.update_entry(pool).await
        } else {
            self.insert_entry(pool).await
        }
    }

    pub async fn update_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                UPDATE heartrate_statistics_summary
                SET min_heartrate=$min_heartrate,max_heartrate=$max_heartrate,
                    mean_heartrate=$mean_heartrate,median_heartrate=$median_heartrate,
                    stdev_heartrate=$stdev_heartrate,number_of_entries=$number_of_entries
                WHERE date=$date
            "#,
            date = self.date,
            min_heartrate = self.min_heartrate,
            max_heartrate = self.max_heartrate,
            mean_heartrate = self.mean_heartrate,
            median_heartrate = self.median_heartrate,
            stdev_heartrate = self.stdev_heartrate,
            number_of_entries = self.number_of_entries,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn insert_entry(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO heartrate_statistics_summary
                (date, min_heartrate, max_heartrate, mean_heartrate, median_heartrate,
                 stdev_heartrate, number_of_entries)
                VALUES
                ($date, $min_heartrate, $max_heartrate, $mean_heartrate, $median_heartrate,
                 $stdev_heartrate, $number_of_entries)
            "#,
            date = self.date,
            min_heartrate = self.min_heartrate,
            max_heartrate = self.max_heartrate,
            mean_heartrate = self.mean_heartrate,
            median_heartrate = self.median_heartrate,
            stdev_heartrate = self.stdev_heartrate,
            number_of_entries = self.number_of_entries,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn get_fitbit_statistics_plots(
        stats: &[Self],
        is_demo: bool,
    ) -> Result<StackString, Error> {
        let template = if is_demo {
            PLOT_TEMPLATE_DEMO
        } else {
            PLOT_TEMPLATE
        };
        if stats.is_empty() {
            let body = template
                .replace("INSERTOTHERIMAGESHERE", "")
                .replace("INSERTTEXTHERE", "")
                .replace("INSERTOTHERTEXTHERE", "")
                .into();
            return Ok(body);
        }
        let mut graphs = Vec::new();
        let start_date = stats[0].date;

        let min_heartrate: Vec<_> = stats
            .iter()
            .map(|stat| {
                let days = (stat.date - start_date).num_days();
                (days as f64, stat.min_heartrate)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("min_heartrate")
            .with_title("Minimum Heartrate")
            .with_data(&min_heartrate)
            .with_labels("days", "Heartrate [bpm]");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let max_heartrate: Vec<_> = stats
            .iter()
            .map(|stat| {
                let days = (stat.date - start_date).num_days();
                (days as f64, stat.max_heartrate)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("max_heartrate")
            .with_title("Maximum Heartrate")
            .with_data(&max_heartrate)
            .with_labels("days", "Heartrate [bpm]");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let mean_heartrate: Vec<_> = stats
            .iter()
            .map(|stat| {
                let days = (stat.date - start_date).num_days();
                (days as f64, stat.mean_heartrate)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("mean_heartrate")
            .with_title("Mean Heartrate")
            .with_data(&mean_heartrate)
            .with_labels("days", "Heartrate [bpm]");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let median_heartrate: Vec<_> = stats
            .iter()
            .map(|stat| {
                let days = (stat.date - start_date).num_days();
                (days as f64, stat.median_heartrate)
            })
            .collect();
        let plot_opt = PlotOpts::new()
            .with_name("median_heartrate")
            .with_title("Median Heartrate")
            .with_data(&median_heartrate)
            .with_labels("days", "Heartrate [bpm]");
        graphs.push(generate_d3_plot(&plot_opt)?);

        let n = stats.len();
        let entries: Vec<_> = stats[n - 10..n]
            .iter()
            .map(|stat| {
                let date = stat.date;
                format!(
                    r#"
                    <td>{}</td><td>{:3.1}</td><td>{:2.1}</td><td>{:2.1}</td>
                    <td>{:2.1}</td>"#,
                    date,
                    stat.min_heartrate,
                    stat.max_heartrate,
                    stat.mean_heartrate,
                    stat.median_heartrate,
                )
            })
            .collect();
        let entries = format!(
            r#"
            <table border=1>
            <thead>
            <th>Date</th><th>Min</th><th>Max</th><th>Mean</th>
            <th>Median</th>
            </thead>
            <tbody>
            <tr>{}</tr>
            </tbody>
            </table>"#,
            entries.join("</tr><tr>")
        );

        let body = template
            .replace("INSERTOTHERIMAGESHERE", &graphs.join("\n"))
            .replace("INSERTTEXTHERE", &entries)
            .replace("INSERTOTHERTEXTHERE", "")
            .into();
        Ok(body)
    }
}
