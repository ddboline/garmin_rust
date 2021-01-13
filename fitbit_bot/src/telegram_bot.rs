use anyhow::Error;
use arc_swap::ArcSwap;
use crossbeam_utils::atomic::AtomicCell;
use futures::StreamExt;
use lazy_static::lazy_static;
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use stack_string::StackString;
use std::{collections::HashSet, sync::Arc};
use telegram_bot::{types::refs::UserId, Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio::{
    task::spawn,
    time::{sleep, Duration},
};

use fitbit_lib::scale_measurement::ScaleMeasurement;
use garmin_lib::common::pgpool::PgPool;

use super::failure_count::FailureCount;

type WeightLock = AtomicCell<Option<ScaleMeasurement>>;
type Userids = ArcSwap<HashSet<UserId>>;

lazy_static! {
    static ref LAST_WEIGHT: WeightLock = AtomicCell::new(None);
    static ref USERIDS: Userids = ArcSwap::new(Arc::new(HashSet::new()));
    static ref FAILURE_COUNT: FailureCount = FailureCount::new(5);
}

#[derive(Clone)]
pub struct TelegramBot {
    telegram_bot_token: StackString,
    pool: PgPool,
}

impl TelegramBot {
    pub fn new(telegram_bot_token: &str, pool: &PgPool) -> Self {
        Self {
            telegram_bot_token: telegram_bot_token.into(),
            pool: pool.clone(),
        }
    }

    pub async fn run_bot(&self) -> Result<(), Error> {
        self.initialize_last_weight().await?;
        let fill_user_ids = {
            let bot = self.clone();
            spawn(async move { bot.fill_telegram_user_ids().await })
        };
        self.telegram_loop().await?;
        fill_user_ids.await?
    }

    async fn telegram_loop(&self) -> Result<(), Error> {
        loop {
            FAILURE_COUNT.check()?;

            match tokio::time::timeout(
                tokio::time::Duration::from_secs(3600),
                self._telegram_worker(),
            )
            .await
            {
                Err(_) | Ok(Ok(_)) => FAILURE_COUNT.reset()?,
                Ok(Err(_)) => FAILURE_COUNT.increment()?,
            }
        }
    }

    async fn _telegram_worker(&self) -> Result<(), Error> {
        let api = Api::new(&self.telegram_bot_token);
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
                    if USERIDS.load().contains(&message.from.id) {
                        FAILURE_COUNT.check()?;
                        if &data.to_lowercase() == "check" {
                            if let Some(meas) = LAST_WEIGHT.load() {
                                api.spawn(
                                    message.text_reply(format!("latest measurement {}", meas)),
                                );
                            } else {
                                api.spawn(message.text_reply("No measurements".to_string()));
                            }
                        } else {
                            match ScaleMeasurement::from_telegram_text(data) {
                                Ok(meas) => match self.process_measurement(meas).await {
                                    Ok(_) => api.spawn(
                                        message.text_reply(format!("sent to the db {}", meas)),
                                    ),
                                    Err(e) => {
                                        api.spawn(message.text_reply(format!("Send Error {}", e)))
                                    }
                                },
                                Err(e) => {
                                    api.spawn(message.text_reply(format!("Parse error {}", e)))
                                }
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

    async fn initialize_last_weight(&self) -> Result<(), Error> {
        let mut last_weight = LAST_WEIGHT.load();
        if let Some(meas) = ScaleMeasurement::read_latest_from_db(&self.pool).await? {
            let current_dt = meas.datetime;
            if let Some(last) = last_weight.replace(meas) {
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

    async fn process_measurement(&self, meas: ScaleMeasurement) -> Result<(), Error> {
        if meas.insert_into_db(&self.pool).await.is_ok() {
            debug!("{:?}", meas);
            LAST_WEIGHT.store(Some(meas));
            FAILURE_COUNT.reset()?;
        } else {
            FAILURE_COUNT.increment()?;
        }
        Ok(())
    }

    async fn fill_telegram_user_ids(&self) -> Result<(), Error> {
        loop {
            FAILURE_COUNT.check()?;
            if let Ok(telegram_userid_set) = self.list_of_telegram_user_ids().await {
                USERIDS.store(Arc::new(telegram_userid_set));
                FAILURE_COUNT.reset()?;
            } else {
                FAILURE_COUNT.increment()?;
            }
            sleep(Duration::from_secs(60)).await;
        }
    }

    async fn list_of_telegram_user_ids(&self) -> Result<HashSet<UserId>, Error> {
        let query = "
        SELECT distinct telegram_userid
        FROM authorized_users
        WHERE telegram_userid IS NOT NULL
    ";
        self.pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .into_par_iter()
            .map(|row| {
                let telegram_userid: i64 = row.try_get("telegram_userid")?;
                Ok(UserId::new(telegram_userid))
            })
            .collect()
    }
}
