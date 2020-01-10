use anyhow::{format_err, Error};
use crossbeam_channel::{unbounded, Receiver, Sender};
use crossbeam_utils::thread::Scope;
use futures::StreamExt;
use lazy_static::lazy_static;
use log::debug;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;
use telegram_bot::types::refs::UserId;
use telegram_bot::{Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio::runtime::Runtime;

use fitbit_lib::scale_measurement::ScaleMeasurement;
use garmin_lib::common::pgpool::PgPool;

type WeightLock = RwLock<Option<ScaleMeasurement>>;
type Userids = RwLock<HashSet<UserId>>;

lazy_static! {
    static ref LAST_WEIGHT: WeightLock = RwLock::new(None);
    static ref USERIDS: Userids = RwLock::new(HashSet::new());
    static ref KILLSWITCH: AtomicBool = AtomicBool::new(false);
}

pub fn run_bot(telegram_bot_token: &str, pool: PgPool, scope: &Scope) -> Result<(), Error> {
    let telegram_bot_token: String = telegram_bot_token.into();
    let (send, recv) = unbounded();

    let pool_ = pool.clone();
    let userid_handle = scope.spawn(move |_| fill_telegram_user_ids(pool_));
    let message_handle = scope.spawn(move |_| process_messages(recv, pool));
    let telegram_handle = scope.spawn(move |_| telegram_worker(&telegram_bot_token, send));

    if userid_handle.join().is_err() {
        panic!("Userid thread paniced, kill everything");
    }
    telegram_handle.join().expect("Telegram handle paniced")?;
    drop(message_handle);
    Ok(())
}

fn telegram_worker(telegram_bot_token: &str, send: Sender<ScaleMeasurement>) -> Result<(), Error> {
    let mut rt = Runtime::new()?;

    rt.block_on(_telegram_worker(telegram_bot_token, send))?;
    KILLSWITCH.store(true, Ordering::SeqCst);
    Ok(())
}

async fn _telegram_worker(
    telegram_bot_token: &str,
    send: Sender<ScaleMeasurement>,
) -> Result<(), Error> {
    let api = Api::new(telegram_bot_token);
    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        if KILLSWITCH.load(Ordering::SeqCst) {
            return Ok(());
        }
        // If the received update contains a new message...
        if let UpdateKind::Message(message) = update?.kind {
            if let MessageKind::Text { ref data, .. } = message.kind {
                // Print received text message to stdout.
                debug!("{:?}", message);
                if USERIDS.read().contains(&message.from.id) {
                    match data.to_lowercase().as_str() {
                        "check" => match *LAST_WEIGHT.read() {
                            Some(meas) => {
                                api.spawn(
                                    message.text_reply(format!("latest measurement {}", meas)),
                                );
                            }
                            None => {
                                api.spawn(message.text_reply("No measurements".to_string()));
                            }
                        },
                        _ => match ScaleMeasurement::from_telegram_text(data) {
                            Ok(meas) => match send.try_send(meas) {
                                Ok(_) => api
                                    .spawn(message.text_reply(format!("sent to the db {}", meas))),
                                Err(e) => {
                                    api.spawn(message.text_reply(format!("Send Error {}", e)))
                                }
                            },
                            Err(e) => api.spawn(message.text_reply(format!("Parse error {}", e))),
                        },
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

fn process_messages(recv: Receiver<ScaleMeasurement>, pool: PgPool) -> Result<(), Error> {
    let meas_list = ScaleMeasurement::read_from_db(&pool, None, None)?;
    let mut last_weight = *LAST_WEIGHT.read();
    for meas in meas_list {
        let current_dt = meas.datetime;
        let last_meas = last_weight.replace(meas);
        if let Some(last) = last_meas {
            if last.datetime > current_dt {
                last_weight.replace(last);
            }
        }
    }
    if let Some(last) = last_weight {
        LAST_WEIGHT.write().replace(last);
    }

    let mut failure_count = 0;
    debug!("LAST_WEIGHT {:?}", *LAST_WEIGHT.read());
    while let Ok(meas) = recv.recv() {
        if KILLSWITCH.load(Ordering::SeqCst) {
            return Ok(());
        }
        if meas.insert_into_db(&pool).is_ok() {
            debug!("{:?}", meas);
            LAST_WEIGHT.write().replace(meas);
            failure_count = 0;
        } else {
            failure_count += 1;
            if failure_count > 5 {
                KILLSWITCH.store(true, Ordering::SeqCst);
                return Err(format_err!(
                    "Failed with {} after retrying {} times",
                    e,
                    failure_count
                ));
            }
        }
    }
    Ok(())
}

fn fill_telegram_user_ids(pool: PgPool) {
    loop {
        if let Ok(telegram_userids) = list_of_telegram_user_ids(&pool) {
            let mut telegram_userid_set = USERIDS.write();
            telegram_userid_set.clear();
            for userid in telegram_userids {
                telegram_userid_set.insert(UserId::new(userid));
            }
        }
        sleep(Duration::from_secs(60));
    }
}

fn list_of_telegram_user_ids(pool: &PgPool) -> Result<Vec<i64>, Error> {
    let query = "
        SELECT distinct telegram_userid
        FROM authorized_users
        WHERE telegram_userid IS NOT NULL
    ";
    pool.get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let telegram_userid: i64 = row.try_get(0)?;
            Ok(telegram_userid)
        })
        .collect()
}
