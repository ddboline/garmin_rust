use futures::Stream;
use itertools::Itertools;
use postgres_query::{query, Error as PqError, FromSqlRow};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
};
use time::{macros::format_description, Date};
use time_tz::OffsetDateTimeExt;
use uuid::Uuid;

use garmin_lib::{date_time_wrapper::DateTimeWrapper, errors::GarminError as Error};
use garmin_models::garmin_summary::GarminSummary;
use garmin_utils::{
    garmin_util::{print_h_m_s, METERS_PER_MILE},
    pgpool::PgPool,
};

use crate::race_type::RaceType;

#[derive(Debug, Clone, Serialize, Deserialize, FromSqlRow, PartialEq)]
pub struct RaceResults {
    pub id: Uuid,
    pub race_type: RaceType,
    pub race_date: Option<Date>,
    pub race_name: Option<StackString>,
    pub race_distance: i32, // distance in meters
    pub race_time: f64,
    pub race_flag: bool,
    pub race_summary_ids: Vec<Option<Uuid>>,
}

impl Display for RaceResults {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let race_date_str = if let Some(date) = self.race_date {
            format_sstr!("\nrace_date: {date}")
        } else {
            StackString::new()
        };
        let race_name_str = if let Some(name) = &self.race_name {
            format_sstr!("\nrace_name: {name}")
        } else {
            StackString::new()
        };
        write!(
            f,
            "RaceResults(\nid: {}\nrace_type: {}{}{}\nrace_distance: {} km\nrace_time: \
             {}\nrace_flag: {}{}\n)",
            self.id,
            self.race_type,
            race_date_str,
            race_name_str,
            self.race_distance,
            print_h_m_s(self.race_time, true).unwrap_or_else(|_| "".into()),
            self.race_flag,
            {
                let summary_ids = self
                    .race_summary_ids
                    .iter()
                    .filter_map(|id| id.map(StackString::from_display))
                    .join(",");

                if summary_ids.is_empty() {
                    StackString::new()
                } else {
                    format_sstr!("summary_ids: {summary_ids}")
                }
            }
        )
    }
}

impl RaceResults {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_results_by_type(
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            "SELECT a.id, a.race_type, a.race_date, a.race_name, a.race_distance, a.race_time,
                    a.race_flag, array_agg(b.summary_id) as race_summary_ids
            FROM race_results a
            LEFT JOIN race_results_garmin_summary b ON a.id = b.race_id
            WHERE a.race_type = $race_type
            GROUP BY 1,2,3,4,5,6,7
            ORDER BY a.race_date, a.race_distance",
            race_type = race_type
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_result_by_id(id: Uuid, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT a.id, a.race_type, a.race_date, a.race_name, a.race_distance, a.race_time,
                    a.race_flag, array_agg(b.summary_id) as race_summary_ids
            FROM race_results a
            LEFT JOIN race_results_garmin_summary b ON a.id = b.race_id
            WHERE a.id = $id
            GROUP BY 1,2,3,4,5,6,7",
            id = id
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_races_by_date(
        race_date: Date,
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = query!(
            "SELECT a.id, a.race_type, a.race_date, a.race_name, a.race_distance, a.race_time,
                    a.race_flag, array_agg(b.summary_id) as race_summary_ids
            FROM race_results a
            LEFT JOIN race_results_garmin_summary b ON a.id = b.race_id
            WHERE a.race_date = $race_date and a.race_type = $race_type
            GROUP BY 1,2,3,4,5,6,7",
            race_date = race_date,
            race_type = race_type,
        );
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_race_by_summary_id(
        summary_id: Uuid,
        pool: &PgPool,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            "SELECT a.id, a.race_type, a.race_date, a.race_name, a.race_distance, a.race_time,
                    a.race_flag, array_agg(b.summary_id) as race_summary_ids
            FROM race_results a
            JOIN race_results_garmin_summary b ON a.id = b.race_id
            WHERE a.id = (
                SELECT b.race_id
                FROM race_results_garmin_summary b
                WHERE b.summary_id = $summary_id
            )
            GROUP BY 1,2,3,4,5,6,7",
            summary_id = summary_id,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_race_by_distance(
        race_distance: i32,
        race_type: RaceType,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = query!(
            "SELECT a.id, a.race_type, a.race_date, a.race_name, a.race_distance, a.race_time,
                    a.race_flag, array_agg(b.summary_id) as race_summary_ids
            FROM race_results a
            LEFT JOIN race_results_garmin_summary b ON a.id = b.race_id
            WHERE a.race_distance = $race_distance and a.race_type = $race_type
            GROUP BY 1,2,3,4,5,6,7",
            race_distance = race_distance,
            race_type = race_type,
        );
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_summary_map(pool: &PgPool) -> Result<HashMap<Uuid, GarminSummary>, Error> {
        let query = "
            SELECT a.*
            FROM garmin_summary a
            JOIN race_results_garmin_summary b ON a.id = b.summary_id
        ";
        let conn = pool.get().await?;
        let mut h = conn
            .query(query, &[])
            .await?
            .into_iter()
            .map(|row| {
                GarminSummary::from_row(&row)
                    .map_err(Into::into)
                    .map(|s| (s.id, s))
            })
            .collect::<Result<HashMap<_, _>, Error>>()?;
        h.shrink_to_fit();
        Ok(h)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_race_id(&self, pool: &PgPool) -> Result<Option<Uuid>, Error> {
        let conn = pool.get().await?;
        let query = match self.race_type {
            RaceType::WorldRecordMen | RaceType::WorldRecordWomen => {
                query!(
                    "
                        SELECT id
                        FROM race_results
                        WHERE race_type = $race_type AND race_distance = $race_distance
                    ",
                    race_type = self.race_type,
                    race_distance = self.race_distance,
                )
            }
            RaceType::Personal => {
                query!(
                    "
                        SELECT id
                        FROM race_results
                        WHERE race_type = $race_type
                          AND race_name = $race_name
                          AND race_date = $race_date
                    ",
                    race_type = self.race_type,
                    race_name = self.race_name,
                    race_date = self.race_date,
                )
            }
        };
        let id: Option<(Uuid,)> = query.fetch_opt(&conn).await?;
        Ok(id.map(|(id,)| id))
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn set_race_id(&mut self, pool: &PgPool) -> Result<(), Error> {
        if let Some(id) = self.get_race_id(pool).await? {
            self.id = id;
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert_into_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                INSERT INTO race_results (
                    race_type, race_date, race_name, race_distance, race_time, race_flag
                )
                VALUES (
                    $race_type, $race_date, $race_name, $race_distance, $race_time, $race_flag
                )
             ",
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn update_db(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "UPDATE race_results
            SET race_type=$race_type,race_date=$race_date,race_name=$race_name,
                race_distance=$race_distance,race_time=$race_time,race_flag=$race_flag
            WHERE id=$id",
            id = self.id,
            race_type = self.race_type,
            race_date = self.race_date,
            race_name = self.race_name,
            race_distance = self.race_distance,
            race_time = self.race_time,
            race_flag = self.race_flag,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;
        self.update_race_summary_ids(pool).await?;
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn upsert_db(&mut self, pool: &PgPool) -> Result<(), Error> {
        if Self::get_result_by_id(self.id, pool).await?.is_some() {
            self.update_db(pool).await?;
        } else {
            self.insert_into_db(pool).await?;
            self.set_race_id(pool).await?;
            self.update_race_summary_ids(pool).await?;
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn update_race_summary_ids(&self, pool: &PgPool) -> Result<(), Error> {
        let summary_ids: SmallVec<[&Uuid; 2]> = self
            .race_summary_ids
            .iter()
            .filter_map(Option::as_ref)
            .take(2)
            .collect();

        if !summary_ids.is_empty() {
            let conn = pool.get().await?;
            for summary_id in summary_ids {
                let query = query!(
                    "
                        INSERT INTO race_results_garmin_summary (race_id, summary_id)
                        VALUES ($race_id, $summary_id)
                        ON CONFLICT DO NOTHING
                    ",
                    race_id = self.id,
                    summary_id = summary_id,
                );
                query.execute(&conn).await?;
            }
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn delete_from_db(self, pool: &PgPool) -> Result<(), Error> {
        let query = query!("DELETE FROM race_results WHERE id = $id", id = self.id);
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub fn parse_from_race_results_text_file(input: &str) -> Result<Vec<Self>, Error> {
        let mut results: Vec<_> = input
            .split('\n')
            .filter_map(|line| {
                let mut entries: Vec<_> = line.split_whitespace().collect();
                entries.shrink_to_fit();
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
                let race_date =
                    Date::parse(entries[4], format_description!("[year]-[month]-[day]")).ok();
                let race_name = entries[5..].join(" ");
                Some(RaceResults {
                    id: Uuid::new_v4(),
                    race_type: RaceType::Personal,
                    race_date,
                    race_name: Some(race_name.into()),
                    race_distance,
                    race_time,
                    race_flag: false,
                    race_summary_ids: Vec::new(),
                })
            })
            .collect();
        results.shrink_to_fit();
        Ok(results)
    }

    #[must_use]
    pub fn parse_world_record_text_file(input: &str, race_type: RaceType) -> Vec<Self> {
        let mut v: Vec<_> = input
            .split('\n')
            .filter_map(|line| {
                let entries: SmallVec<[&str; 2]> = line.split_whitespace().take(2).collect();
                let distance: f64 = match entries.first().and_then(|e| e.parse().ok()) {
                    Some(d) => d,
                    None => return None,
                };
                let race_distance = (distance * 1000.0) as i32;
                let race_time: f64 = match entries.get(1).and_then(|e| parse_time_string(e)) {
                    Some(t) => t,
                    None => return None,
                };
                Some(RaceResults {
                    id: Uuid::new_v4(),
                    race_type,
                    race_date: None,
                    race_name: None,
                    race_distance,
                    race_time,
                    race_flag: false,
                    race_summary_ids: Vec::new(),
                })
            })
            .collect();
        v.shrink_to_fit();
        v
    }
}

fn parse_time_string(s: &str) -> Option<f64> {
    let times: SmallVec<[&str; 3]> = s.split(':').rev().take(3).collect();

    let seconds: f64 = times.first().and_then(|t| t.parse().ok())?;

    let minutes: f64 = times.get(1).and_then(|t| t.parse().ok())?;

    let hours: f64 = match times.get(2).and_then(|t| t.parse().ok()) {
        Some(t) => t,
        None => return Some(minutes * 60.0 + seconds),
    };
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

impl From<GarminSummary> for RaceResults {
    fn from(item: GarminSummary) -> Self {
        let local = DateTimeWrapper::local_tz();
        Self {
            id: Uuid::new_v4(),
            race_type: RaceType::Personal,
            race_date: Some(item.begin_datetime.to_timezone(local).date()),
            race_name: None,
            race_distance: item.total_distance as i32,
            race_time: item.total_duration,
            race_flag: false,
            race_summary_ids: vec![Some(item.id)],
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;
    use log::debug;
    use std::sync::LazyLock;
    use parking_lot::Mutex;
    use stack_string::format_sstr;
    use std::collections::HashMap;
    use time::{macros::date, OffsetDateTime};
    use uuid::Uuid;

    use garmin_lib::{errors::GarminError as Error, garmin_config::GarminConfig};
    use garmin_models::garmin_summary::{get_list_of_files_from_db, GarminSummary};
    use garmin_utils::pgpool::PgPool;

    use crate::{race_results::RaceResults, race_type::RaceType};

    const WORLD_RECORD_ENTRIES: usize = 24;
    const TEST_RACE_ENTRIES: usize = 214;

    static DB_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn get_test_race_result() -> RaceResults {
        RaceResults {
            id: Uuid::new_v4(),
            race_type: RaceType::Personal,
            race_date: Some(date!(2020 - 01 - 27)),
            race_name: Some("A Test Race".into()),
            race_distance: 5000,
            race_time: 1563.0,
            race_flag: false,
            race_summary_ids: Vec::new(),
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
        let pool = PgPool::new(&config.pgurl)?;
        let result = get_test_race_result();
        debug!("{:?}", result);
        result.insert_into_db(&pool).await?;

        let db_result =
            RaceResults::get_races_by_date(date!(2020 - 01 - 27), RaceType::Personal, &pool)
                .await?;
        debug!("{:?}", db_result);
        assert_eq!(db_result.len(), 1);
        for r in db_result {
            r.delete_from_db(&pool).await?;
        }
        let db_result =
            RaceResults::get_races_by_date(date!(2020 - 01 - 27), RaceType::Personal, &pool)
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
        let pool = PgPool::new(&config.pgurl)?;

        let existing_map: HashMap<_, _> =
            RaceResults::get_results_by_type(RaceType::Personal, &pool)
                .await?
                .try_fold(HashMap::new(), |mut map, result| async move {
                    let name = result.race_name.clone().unwrap_or_else(|| "".into());
                    let year = result
                        .race_date
                        .map_or_else(|| OffsetDateTime::now_utc().year(), |d| d.year());
                    let key = (name, year);
                    map.entry(key).or_insert_with(Vec::new).push(result);
                    Ok(map)
                })
                .await?;

        for result in &new_results {
            let name = result.race_name.clone().unwrap_or_else(|| "".into());
            let year = result
                .race_date
                .map_or_else(|| OffsetDateTime::now_utc().year(), |d| d.year());
            let key = (name, year);
            if !existing_map.contains_key(&key) {
                result.insert_into_db(&pool).await?;
            }
        }

        let mut personal_results: Vec<_> =
            RaceResults::get_results_by_type(RaceType::Personal, &pool)
                .await?
                .try_collect()
                .await?;
        personal_results.shrink_to_fit();
        assert!(personal_results.len() >= TEST_RACE_ENTRIES);

        let mut existing_map: HashMap<_, _> =
            personal_results
                .into_iter()
                .fold(HashMap::new(), |mut map, result| {
                    let name = result.race_name.clone().unwrap_or_else(|| "".into());
                    let year = result
                        .race_date
                        .map_or_else(|| OffsetDateTime::now_utc().year(), |d| d.year());
                    let key = (name, year);
                    map.entry(key).or_insert_with(Vec::new).push(result);
                    map
                });
        let input = include_str!("../../tests/data/running_paces_backup1.txt");
        for line in input.split('\n') {
            let mut entries: Vec<_> = line.split_whitespace().collect();
            entries.shrink_to_fit();
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
                    // race.upsert_db(&pool).await?;
                }
            } else {
                assert!(false, "No existing entry {:?}", key);
            }
        }

        let mut personal_results: Vec<_> =
            RaceResults::get_results_by_type(RaceType::Personal, &pool)
                .await?
                .try_collect()
                .await?;
        personal_results.shrink_to_fit();
        assert!(personal_results.len() >= TEST_RACE_ENTRIES);

        for mut result in personal_results {
            if result
                .race_summary_ids
                .iter()
                .filter_map(|i| i.as_ref())
                .count()
                > 0
            {
                continue;
            }
            if let Some(race_date) = result.race_date {
                let constraint = format_sstr!(
                    "replace({}, '%', 'T') like '{}%'",
                    "to_char(begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                    race_date,
                );
                let mut filenames: Vec<_> = get_list_of_files_from_db(&constraint, &pool)
                    .await?
                    .try_collect()
                    .await?;
                filenames.shrink_to_fit();
                if filenames.is_empty() {
                    continue;
                }
                for filename in filenames {
                    if let Some(summary) =
                        GarminSummary::get_by_filename(&pool, filename.as_str()).await?
                    {
                        if (summary.total_distance as i32 - result.race_distance).abs() < 4000 {
                            debug!("set filename: {}", filename);
                            debug!("{:?}", result);
                            debug!("{:?}", summary);
                            result.race_summary_ids.push(Some(summary.id));
                            result.upsert_db(&pool).await?;
                        } else {
                            debug!(
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
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_parse_world_record_text_file() -> Result<(), Error> {
        let mens = include_str!("../../tests/data/running_world_records_men.txt");
        let mens_results =
            RaceResults::parse_world_record_text_file(&mens, RaceType::WorldRecordMen);
        let womens = include_str!("../../tests/data/running_world_records_women.txt");
        let womens_results =
            RaceResults::parse_world_record_text_file(&womens, RaceType::WorldRecordWomen);
        assert_eq!(mens_results.len(), 24);
        assert_eq!(womens_results.len(), 24);

        let _ = DB_LOCK.lock();

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl)?;

        for mut result in mens_results.into_iter().chain(womens_results.into_iter()) {
            let existing =
                RaceResults::get_race_by_distance(result.race_distance, result.race_type, &pool)
                    .await?;
            assert!(existing.len() == 0 || existing.len() == 1, "{:?}", existing);
            if existing.len() == 1 {
                result.id = existing[0].id;
                result.race_flag = true;
            }
            result.upsert_db(&pool).await?;
        }
        let mut mens_results: Vec<_> =
            RaceResults::get_results_by_type(RaceType::WorldRecordMen, &pool)
                .await?
                .try_collect()
                .await?;
        mens_results.shrink_to_fit();
        assert_eq!(mens_results.len(), WORLD_RECORD_ENTRIES);
        let mut womens_results: Vec<_> =
            RaceResults::get_results_by_type(RaceType::WorldRecordWomen, &pool)
                .await?
                .try_collect()
                .await?;
        womens_results.shrink_to_fit();
        assert_eq!(womens_results.len(), WORLD_RECORD_ENTRIES);
        Ok(())
    }
}
