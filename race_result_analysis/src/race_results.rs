use anyhow::Error;
use chrono::NaiveDate;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

use stack_string::StackString;

use garmin_lib::{
    common::{garmin_summary::GarminSummary, pgpool::PgPool},
    utils::garmin_util::{print_h_m_s, METERS_PER_MILE},
};

use crate::race_type::RaceType;

#[derive(Debug, Clone, Serialize, Deserialize, FromSqlRow, PartialEq)]
pub struct RaceResults {
    pub id: i32,
    pub race_type: RaceType,
    pub race_date: Option<NaiveDate>,
    pub race_name: Option<StackString>,
    pub race_distance: i32, // distance in meters
    pub race_time: f64,
    pub race_flag: bool,
    pub race_filename: Option<StackString>,
}

impl Display for RaceResults {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RaceResults(\nid: {}\nrace_type: {}{}{}\nrace_distance: {} km\nrace_time: \
             {}\nrace_flag: {}{}\n)",
            self.id,
            self.race_type,
            if let Some(date) = self.race_date {
                format!("\nrace_date: {}", date)
            } else {
                "".to_string()
            },
            if let Some(name) = &self.race_name {
                format!("\nrace_name: {}", name)
            } else {
                "".to_string()
            },
            self.race_distance,
            print_h_m_s(self.race_time, true).unwrap_or_else(|_| "".into()),
            self.race_flag,
            if let Some(filename) = &self.race_filename {
                format!("\nrace_filename: {}", filename)
            } else {
                "".to_string()
            }
        )
    }
}

impl RaceResults {
    pub async fn get_results_by_type(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_type = $race_type",
            race_type = race_type
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn get_result_by_id(id: i32, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!("SELECT * FROM race_results WHERE id = $id", id = id);
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    pub async fn get_race_by_date(
        race_date: NaiveDate,
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_date = $race_date and race_type = $race_type",
            race_date = race_date,
            race_type = race_type,
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn get_race_by_filename(
        race_filename: &str,
        pool: &PgPool,
    ) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_filename = $race_filename",
            race_filename = race_filename,
        );
        let conn = pool.get().await?;
        let result = conn
            .query_opt(query.sql(), query.parameters())
            .await?
            .map(|row| Self::from_row(&row))
            .transpose()?;
        Ok(result)
    }

    pub async fn get_race_by_distance(
        race_distance: i32,
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT * FROM race_results WHERE race_distance = $race_distance and race_type = \
             $race_type",
            race_distance = race_distance,
            race_type = race_type,
        );
        let conn = pool.get().await?;
        conn.query(query.sql(), query.parameters())
            .await?
            .into_iter()
            .map(|row| Self::from_row(&row).map_err(Into::into))
            .collect()
    }

    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "
            INSERT INTO race_results (race_type, race_date, race_name, race_distance, race_time,
                race_flag,race_filename)
            VALUES ($race_type,$race_date,$race_name,$race_distance,$race_time,$race_flag,$race_filename)",
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
            race_filename = self.race_filename,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "UPDATE race_results
            SET race_type=$race_type,race_date=$race_date,race_name=$race_name,
                race_distance=$race_distance,race_time=$race_time,race_flag=$race_flag,
                race_filename=$race_filename
            WHERE id=$id",
            id = self.id,
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
            race_filename = self.race_filename,
        );
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn upsert_db(&self, pool: &PgPool) -> Result<(), Error> {
        if Self::get_result_by_id(self.id, pool).await?.is_some() {
            self.update_db(pool).await?;
        } else {
            self.insert_into_db(pool).await?;
        }
        Ok(())
    }

    pub async fn delete_from_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!("DELETE FROM race_results WHERE id = $id", id = self.id);
        let conn = pool.get().await?;
        conn.execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn parse_from_race_results_text_file(input: &str) -> Result<Vec<Self>, Error> {
        let results: Vec<_> = input
            .split('\n')
            .filter_map(|line| {
                let entries: Vec<_> = line.split_whitespace().collect();
                if entries.len() < 6 {
                    return None;
                }
                let distance: f64 = match entries[0].parse() {
                    Ok(d) => d,
                    Err(_) => return None,
                };
                let race_distance: i32 = match entries.get(1) {
                    Some(&"km") => (distance * 1000.0) as i32,
                    Some(&"mi") => (distance * METERS_PER_MILE) as i32,
                    _ => return None,
                };
                let race_time: f64 = match parse_time_string(entries[2]) {
                    Some(t) => t,
                    None => return None,
                };
                let race_date = entries[4].parse().ok();
                let race_name = entries[5..].join(" ");
                Some(RaceResults {
                    id: -1,
                    race_type: RaceType::Personal,
                    race_date,
                    race_name: Some(race_name.into()),
                    race_distance,
                    race_time,
                    race_flag: false,
                    race_filename: None,
                })
            })
            .collect();
        Ok(results)
    }

    pub fn parse_world_record_text_file(
        input: &str,
        race_type: RaceType,
    ) -> Result<Vec<Self>, Error> {
        let results: Vec<_> = input
            .split('\n')
            .filter_map(|line| {
                let mut entries = line.split_whitespace();
                let distance: f64 = match entries.next().and_then(|e| e.parse().ok()) {
                    Some(d) => d,
                    None => return None,
                };
                let race_distance = (distance * 1000.0) as i32;
                let race_time: f64 = match entries.next().and_then(|e| parse_time_string(e)) {
                    Some(t) => t,
                    None => return None,
                };
                Some(RaceResults {
                    id: -1,
                    race_type,
                    race_date: None,
                    race_name: None,
                    race_distance,
                    race_time,
                    race_flag: false,
                    race_filename: None,
                })
            })
            .collect();
        Ok(results)
    }
}

fn parse_time_string(s: &str) -> Option<f64> {
    let mut times = s.split(':').rev();

    let seconds: f64 = match times.next().and_then(|t| t.parse().ok()) {
        Some(t) => t,
        None => return None,
    };

    let minutes: f64 = match times.next().and_then(|t| t.parse().ok()) {
        Some(t) => t,
        None => return None,
    };

    let hours: f64 = match times.next().and_then(|t| t.parse().ok()) {
        Some(t) => t,
        None => return Some(minutes * 60.0 + seconds),
    };
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

impl From<GarminSummary> for RaceResults {
    fn from(item: GarminSummary) -> Self {
        Self {
            id: -1,
            race_type: RaceType::Personal,
            race_date: Some(item.begin_datetime.naive_local().date()),
            race_name: None,
            race_distance: item.total_distance as i32,
            race_time: item.total_duration,
            race_flag: false,
            race_filename: Some(item.filename),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{Datelike, NaiveDate, Utc};
    use lazy_static::lazy_static;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    use garmin_lib::common::{
        garmin_config::GarminConfig,
        garmin_summary::{get_list_of_files_from_db, GarminSummary},
        pgpool::PgPool,
    };

    use crate::{race_results::RaceResults, race_type::RaceType};

    const WORLD_RECORD_ENTRIES: usize = 24;
    const TEST_RACE_ENTRIES: usize = 214;

    lazy_static! {
        static ref DB_LOCK: Mutex<()> = Mutex::new(());
    }

    fn get_test_race_result() -> RaceResults {
        RaceResults {
            id: 0,
            race_type: RaceType::Personal,
            race_date: Some(NaiveDate::from_ymd(2020, 1, 27)),
            race_name: Some("A Test Race".into()),
            race_distance: 5000,
            race_time: 1563.0,
            race_flag: false,
            race_filename: None,
        }
    }

    #[test]
    fn test_race_results_display() -> Result<(), Error> {
        let result = get_test_race_result();
        let result = result.to_string();
        assert!(result.contains("race_time: 00:26:03"));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_insert_delete_entry() -> Result<(), Error> {
        let _ = DB_LOCK.lock();

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let result = get_test_race_result();
        result.insert_into_db(&pool).await?;

        let db_result = RaceResults::get_race_by_date(
            NaiveDate::from_ymd(2020, 1, 27),
            RaceType::Personal,
            &pool,
        )
        .await?;
        println!("{:?}", db_result);
        assert_eq!(db_result.len(), 1);
        for r in db_result {
            r.delete_from_db(&pool).await?;
        }
        let db_result = RaceResults::get_race_by_date(
            NaiveDate::from_ymd(2020, 1, 27),
            RaceType::Personal,
            &pool,
        )
        .await?;
        assert_eq!(db_result.len(), 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_parse_from_race_results_text_file() -> Result<(), Error> {
        let input = include_str!("../../tests/data/Race_Results.txt");

        let new_results = RaceResults::parse_from_race_results_text_file(&input)?;
        assert_eq!(new_results.len(), TEST_RACE_ENTRIES);

        let _ = DB_LOCK.lock();

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        let existing_map: HashMap<_, _> =
            RaceResults::get_results_by_type(RaceType::Personal, &pool)
                .await?
                .into_iter()
                .fold(HashMap::new(), |mut map, result| {
                    let name = result.race_name.clone().unwrap_or_else(|| "".into());
                    let year = result
                        .race_date
                        .map_or_else(|| Utc::now().year(), |d| d.year());
                    let key = (name, year);
                    map.entry(key).or_insert_with(Vec::new).push(result);
                    map
                });

        for result in &new_results {
            let name = result.race_name.clone().unwrap_or_else(|| "".into());
            let year = result
                .race_date
                .map_or_else(|| Utc::now().year(), |d| d.year());
            let key = (name, year);
            if !existing_map.contains_key(&key) {
                result.insert_into_db(&pool).await?;
            }
        }

        let personal_results = RaceResults::get_results_by_type(RaceType::Personal, &pool).await?;
        assert_eq!(personal_results.len(), TEST_RACE_ENTRIES);

        let mut existing_map: HashMap<_, _> =
            personal_results
                .into_iter()
                .fold(HashMap::new(), |mut map, result| {
                    let name = result.race_name.clone().unwrap_or_else(|| "".into());
                    let year = result
                        .race_date
                        .map_or_else(|| Utc::now().year(), |d| d.year());
                    let key = (name, year);
                    map.entry(key).or_insert_with(Vec::new).push(result);
                    map
                });
        let input = include_str!("../../tests/data/running_paces_backup1.txt");
        for line in input.split('\n') {
            let entries: Vec<_> = line.split_whitespace().collect();
            if entries.len() < 6 {
                continue;
            }
            let year: i32 = entries[3].parse()?;
            let race_flag: u8 = entries[4].parse()?;
            let title = entries[5..].join(" ");
            let key = (title.into(), year);
            if let Some(races) = existing_map.get_mut(&key) {
                assert!(races.len() == 1);
                if race_flag != 1 {
                    continue;
                }
                for race in races.iter_mut() {
                    race.race_flag = true;
                    race.upsert_db(&pool).await?;
                }
            } else {
                assert!(false, "No existing entry {:?}", key);
            }
        }

        let personal_results = RaceResults::get_results_by_type(RaceType::Personal, &pool).await?;
        assert_eq!(personal_results.len(), TEST_RACE_ENTRIES);

        for mut result in personal_results {
            if result.race_filename.is_some() {
                continue;
            }
            if let Some(race_date) = result.race_date {
                let constraint = format!(
                    "replace({}, '%', 'T') like '{}%'",
                    "to_char(begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                    race_date,
                );
                let filenames = get_list_of_files_from_db(&constraint, &pool).await?;
                if filenames.is_empty() {
                    continue;
                }
                for filename in filenames {
                    if let Some(summary) =
                        GarminSummary::get_by_filename(&pool, filename.as_str()).await?
                    {
                        if (summary.total_distance as i32 - result.race_distance).abs() < 4000 {
                            println!("set filename: {}", filename);
                            result.race_filename = Some(filename);
                            result.upsert_db(&pool).await?;
                        } else {
                            println!(
                                "{} difference {} {} {} {}",
                                race_date,
                                summary.total_distance as i32,
                                result.race_distance,
                                (summary.total_distance as i32 - result.race_distance).abs(),
                                filename,
                            );
                        }
                    }
                }
            }
        }
        assert!(false);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_parse_world_record_text_file() -> Result<(), Error> {
        let mens = include_str!("../../tests/data/running_world_records_men.txt");
        let mens_results =
            RaceResults::parse_world_record_text_file(&mens, RaceType::WorldRecordMen)?;
        let womens = include_str!("../../tests/data/running_world_records_women.txt");
        let womens_results =
            RaceResults::parse_world_record_text_file(&womens, RaceType::WorldRecordWomen)?;
        assert_eq!(mens_results.len(), 24);
        assert_eq!(womens_results.len(), 24);

        let _ = DB_LOCK.lock();

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        for mut result in mens_results.into_iter().chain(womens_results.into_iter()) {
            let existing =
                RaceResults::get_race_by_distance(result.race_distance, result.race_type, &pool)
                    .await?;
            assert!(existing.len() == 0 || existing.len() == 1);
            if existing.len() == 1 {
                result.id = existing[0].id;
                result.race_flag = true;
            }
            result.upsert_db(&pool).await?;
        }
        let mens_results =
            RaceResults::get_results_by_type(RaceType::WorldRecordMen, &pool).await?;
        assert_eq!(mens_results.len(), WORLD_RECORD_ENTRIES);
        let womens_results =
            RaceResults::get_results_by_type(RaceType::WorldRecordWomen, &pool).await?;
        assert_eq!(womens_results.len(), WORLD_RECORD_ENTRIES);
        Ok(())
    }
}
