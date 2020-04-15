use anyhow::{format_err, Error};
use crossbeam_utils::atomic::AtomicCell;
use futures::StreamExt;
use lazy_static::lazy_static;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
};
use telegram_bot::{types::refs::UserId, Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio::{
    sync::RwLock,
    task::spawn,
    time::{delay_for, Duration},
};

use fitbit_lib::scale_measurement::ScaleMeasurement;
use garmin_lib::common::pgpool::PgPool;

type WeightLock = AtomicCell<Option<ScaleMeasurement>>;
type Userids = RwLock<HashSet<UserId>>;

lazy_static! {
    static ref LAST_WEIGHT: WeightLock = AtomicCell::new(None);
    static ref USERIDS: Userids = RwLock::new(HashSet::new());
    static ref FAILURE_COUNT: FailureCount = FailureCount::new(5);
}

struct FailureCount {
    max_count: usize,
    counter: AtomicUsize,
}

impl FailureCount {
    fn new(max_count: usize) -> Self {
        Self {
            max_count,
            counter: AtomicUsize::new(0),
        }
    }

    fn check(&self) -> Result<(), Error> {
        if self.counter.load(Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }

    fn reset(&self) -> Result<(), Error> {
        if self.counter.swap(0, Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }

    fn increment(&self) -> Result<(), Error> {
        if self.counter.fetch_add(1, Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }
}

pub async fn run_bot(telegram_bot_token: &str, pool: PgPool) -> Result<(), Error> {
    initialize_last_weight(&pool).await?;
    let pool_ = pool.clone();
    let fill_user_ids = spawn(fill_telegram_user_ids(pool_));
    telegram_loop(&telegram_bot_token, &pool).await?;
    fill_user_ids.await?
}

async fn telegram_loop(telegram_bot_token: &str, pool: &PgPool) -> Result<(), Error> {
    loop {
        FAILURE_COUNT.check()?;

        match tokio::time::timeout(
            tokio::time::Duration::from_secs(3600),
            _telegram_worker(telegram_bot_token, pool),
        )
        .await
        {
            Err(_) | Ok(Ok(_)) => FAILURE_COUNT.reset()?,
            Ok(Err(_)) => FAILURE_COUNT.increment()?,
        }
    }
}

async fn _telegram_worker(telegram_bot_token: &str, pool: &PgPool) -> Result<(), Error> {
    let api = Api::new(telegram_bot_token);
    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        FAILURE_COUNT.check()?;
        // If the received update contains a new message...
        if let UpdateKind::Message(message) = update?.kind {
            FAILURE_COUNT.check()?;
            if let MessageKind::Text { ref data, .. } = message.kind {
                FAILURE_COUNT.check()?;
                // Print received text message to stdout.
                debug!("{:?}", message);
                if USERIDS.read().await.contains(&message.from.id) {
                    FAILURE_COUNT.check()?;
                    if &data.to_lowercase() == "check" {
                        if let Some(meas) = LAST_WEIGHT.load() {
                            api.spawn(message.text_reply(format!("latest measurement {}", meas)));
                        } else {
                            api.spawn(message.text_reply("No measurements".to_string()));
                        }
                    } else {
                        match ScaleMeasurement::from_telegram_text(data) {
                            Ok(meas) => match process_measurement(&meas, pool).await {
                                Ok(_) => api
                                    .spawn(message.text_reply(format!("sent to the db {}", meas))),
                                Err(e) => {
                                    api.spawn(message.text_reply(format!("Send Error {}", e)))
                                }
                            },
                            Err(e) => api.spawn(message.text_reply(format!("Parse error {}", e))),
                        }
                    }
                } else {
                    // Answer message with "Hi".
                    api.spawn(message.text_reply(format!(
                        "Hi, {}, user_id {}! You just wrote '{}'",
                        &message.from.first_name, &message.from.id, data
                    )));
                }
            }
        }
    }
    Ok(())
}

async fn initialize_last_weight(pool: &PgPool) -> Result<(), Error> {
    let meas_list = ScaleMeasurement::read_from_db(&pool, None, None).await?;
    let mut last_weight = LAST_WEIGHT.load();
    for meas in meas_list {
        let current_dt = meas.datetime;
        let last_meas = last_weight.replace(meas);
        if let Some(last) = last_meas {
            if last.datetime > current_dt {
                last_weight.replace(last);
            }
        }
    }
    if last_weight.is_some() {
        LAST_WEIGHT.store(last_weight);
    }
    Ok(())
}

async fn process_measurement(meas: &ScaleMeasurement, pool: &PgPool) -> Result<(), Error> {
    if meas.insert_into_db(pool).await.is_ok() {
        debug!("{:?}", meas);
        LAST_WEIGHT.store(Some(*meas));
        FAILURE_COUNT.reset()?;
    } else {
        FAILURE_COUNT.increment()?;
    }
    Ok(())
}

async fn fill_telegram_user_ids(pool: PgPool) -> Result<(), Error> {
    loop {
        FAILURE_COUNT.check()?;
        if let Ok(telegram_userids) = list_of_telegram_user_ids(&pool).await {
            let telegram_userid_set: HashSet<_> =
                telegram_userids.into_iter().map(UserId::new).collect();
            *USERIDS.write().await = telegram_userid_set;
            FAILURE_COUNT.reset()?;
        } else {
            FAILURE_COUNT.increment()?;
        }
        delay_for(Duration::from_secs(60)).await;
    }
}

async fn list_of_telegram_user_ids(pool: &PgPool) -> Result<Vec<i64>, Error> {
    let query = "
        SELECT distinct telegram_userid
        FROM authorized_users
        WHERE telegram_userid IS NOT NULL
    ";
    pool.get()
        .await?
        .query(query, &[])
        .await?
        .into_par_iter()
        .map(|row| {
            let telegram_userid: i64 = row.try_get("telegram_userid")?;
            Ok(telegram_userid)
        })
        .collect()
}
