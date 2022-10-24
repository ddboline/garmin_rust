use anyhow::Error;
use arc_swap::ArcSwap;
use crossbeam_utils::atomic::AtomicCell;
use futures::StreamExt;
use lazy_static::lazy_static;
use log::debug;
use stack_string::{format_sstr, StackString};
use std::{collections::HashSet, sync::Arc};
use telegram_bot::{
    types::refs::UserId, Api, CanReplySendMessage, Message, MessageKind, Update, UpdateKind,
};
use tokio::{
    task::spawn,
    time::{sleep, Duration},
};

use fitbit_lib::scale_measurement::ScaleMeasurement;
use garmin_lib::{
    common::{garmin_config::GarminConfig, pgpool::PgPool},
    utils::garmin_util::get_list_of_telegram_userids,
};

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
    config: GarminConfig,
}

impl TelegramBot {
    pub fn new(
        telegram_bot_token: impl Into<StackString>,
        pool: &PgPool,
        config: &GarminConfig,
    ) -> Self {
        Self {
            telegram_bot_token: telegram_bot_token.into(),
            pool: pool.clone(),
            config: config.clone(),
        }
    }

    /// # Errors
    /// Returns error if bot call fails
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
            self.process_update(
                |message, s| api.spawn(message.text_reply(s.as_str())),
                update?,
            )
            .await?;
        }
        Ok(())
    }

    async fn process_update(
        &self,
        func: impl Fn(&Message, StackString),
        update: Update,
    ) -> Result<(), Error> {
        if let UpdateKind::Message(message) = update.kind {
            FAILURE_COUNT.check()?;
            if let MessageKind::Text { ref data, .. } = message.kind {
                FAILURE_COUNT.check()?;
                // Print received text message to stdout.
                debug!("{:?}", message);

                func(
                    &message,
                    self.process_message_text(data, &message.from.first_name, message.from.id)
                        .await?,
                );
            }
        }
        Ok(())
    }

    async fn process_message_text(
        &self,
        data: &str,
        first_name: &str,
        user_id: UserId,
    ) -> Result<StackString, Error> {
        if USERIDS.load().contains(&user_id) {
            FAILURE_COUNT.check()?;
            if &data.to_lowercase() == "check" {
                if let Some(meas) = LAST_WEIGHT.load() {
                    Ok(format_sstr!(
                        "latest measurement {meas}, bmi {:2.1}",
                        meas.get_bmi(&self.config)
                    ))
                } else {
                    Ok("No measurements".into())
                }
            } else {
                match ScaleMeasurement::from_telegram_text(data) {
                    Ok(meas) => match self.process_measurement(meas).await {
                        Ok(meas) => Ok(format_sstr!(
                            "sent to the db {meas}, bmi {:2.1}",
                            meas.get_bmi(&self.config)
                        )),
                        Err(e) => Ok(format_sstr!("Send Error {e}")),
                    },
                    Err(e) => Ok(format_sstr!("Parse error {e}")),
                }
            }
        } else {
            // Answer message with "Hi".
            Ok(format_sstr!(
                "Hi, {first_name}, user_id {user_id}! You just wrote '{data}'"
            ))
        }
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

    async fn process_measurement(
        &self,
        mut meas: ScaleMeasurement,
    ) -> Result<ScaleMeasurement, Error> {
        if meas.insert_into_db(&self.pool).await.is_ok() {
            debug!("{:?}", meas);
            LAST_WEIGHT.store(Some(meas));
            FAILURE_COUNT.reset()?;
        } else {
            FAILURE_COUNT.increment()?;
        }
        Ok(meas)
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
        let result = get_list_of_telegram_userids(&self.pool)
            .await?
            .into_iter()
            .map(UserId::new)
            .collect();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use lazy_static::lazy_static;
    use maplit::hashset;
    use parking_lot::Mutex;
    use postgres_query::query;
    use rand::{distributions::Alphanumeric, thread_rng, Rng};
    use stack_string::{format_sstr, StackString};
    use std::{collections::HashSet, sync::Arc};
    use telegram_bot::UserId;
    use uuid::Uuid;

    use fitbit_lib::scale_measurement::ScaleMeasurement;
    use garmin_lib::{
        common::{garmin_config::GarminConfig, pgpool::PgPool},
        utils::date_time_wrapper::iso8601::convert_datetime_to_str,
    };

    use crate::telegram_bot::{TelegramBot, LAST_WEIGHT, USERIDS};

    lazy_static! {
        static ref DB_LOCK: Mutex<()> = Mutex::new(());
    }

    #[tokio::test]
    async fn test_process_message_text() -> Result<(), Error> {
        let _lock = DB_LOCK.lock();

        LAST_WEIGHT.store(None);

        let message = "Hey, what does this do?";
        let user: UserId = 8675309.into();

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);

        let bot = TelegramBot::new("8675309", &pool, &config);

        let result = bot.process_message_text(&message, "User", user).await?;

        assert_eq!(
            result,
            "Hi, User, user_id 8675309! You just wrote \'Hey, what does this do?\'".to_string()
        );

        let telegram_ids: HashSet<UserId> = hashset! { user };
        USERIDS.store(Arc::new(telegram_ids));

        let result = bot.process_message_text("check", "User", user).await?;

        assert_eq!(result, "No measurements".to_string());

        let result = bot.process_message_text(&message, "User", user).await?;

        assert_eq!(
            result,
            "Parse error invalid digit found in string".to_string()
        );

        let msg = "1880=206=596=404=42";
        let obs = ScaleMeasurement::from_telegram_text(msg)?;

        LAST_WEIGHT.store(Some(obs));

        let exp = format_sstr!(
            "ScaleMeasurement(\nid: -1\ndatetime: {}\nmass: 188 lbs\nfat: 20.6%\nwater: \
             59.6%\nmuscle: 40.4%\nbone: 4.2%\n)",
            convert_datetime_to_str(obs.datetime.into())
        );

        assert_eq!(StackString::from_display(obs), exp);

        let result = bot.process_message_text("check", "User", user).await?;

        assert_eq!(result, format_sstr!("latest measurement {obs}"));

        let result = bot
            .process_message_text("1880=206=596=404=42", "User", user)
            .await?;

        for line in result.split('\n').filter(|x| x.contains("id: ")) {
            let id: Uuid = line.trim().replace("id: ", "").parse()?;
            println!("{}", id);
            let obj = ScaleMeasurement::get_by_id(id, &pool).await?.unwrap();
            obj.delete_from_db(&pool).await?;
        }

        assert!(result.starts_with("sent to the db "));
        Ok(())
    }

    #[tokio::test]
    async fn test_initialize_last_weight() -> Result<(), Error> {
        let _lock = DB_LOCK.lock();

        let msg = "1880=206=596=404=42";
        let mut exp = ScaleMeasurement::from_telegram_text(msg)?;

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let bot = TelegramBot::new("8675309", &pool, &config);

        exp.insert_into_db(&pool).await?;
        bot.initialize_last_weight().await?;

        let obs = LAST_WEIGHT.load().unwrap();

        assert_eq!(obs.to_string(), exp.to_string());

        exp.delete_from_db(&pool).await?;

        Ok(())
    }

    fn get_random_string(size: usize) -> String {
        let mut rng = thread_rng();
        (0..size)
            .map(|_| char::from(rng.sample(Alphanumeric)))
            .collect()
    }

    #[tokio::test]
    async fn test_list_of_telegram_user_ids() -> Result<(), Error> {
        let _lock = DB_LOCK.lock();

        USERIDS.store(Arc::new(HashSet::new()));

        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let bot = TelegramBot::new("8675309", &pool, &config);

        let email = format_sstr!("user{}@localhost", get_random_string(32));
        let userid: UserId = 8675309.into();

        let original_user_ids = bot.list_of_telegram_user_ids().await?;

        assert!(!original_user_ids.contains(&userid));

        let query = query!(
            "INSERT INTO authorized_users (email, telegram_userid)
            VALUES ($email, $telegram_userid)",
            email = email,
            telegram_userid = 8675309i64,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await?;

        let new_user_ids = bot.list_of_telegram_user_ids().await?;

        assert!(new_user_ids.contains(&userid));

        let query = query!(
            "DELETE FROM authorized_users WHERE email = $email",
            email = email
        );
        query.execute(&conn).await?;
        Ok(())
    }
}
