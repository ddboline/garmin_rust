use crossbeam_channel::{unbounded, Receiver};
use crossbeam_utils::thread::Scope;
use failure::{format_err, Error};
use futures::Stream;
use lazy_static::lazy_static;
use log::debug;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use telegram_bot::types::refs::UserId;
use telegram_bot::{Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio_core::reactor::Core;

use crate::scale_measurement::ScaleMeasurement;
use garmin_lib::common::pgpool::PgPool;
use garmin_lib::utils::row_index_trait::RowIndexTrait;

lazy_static! {
    static ref LAST_WEIGHT: Arc<RwLock<Option<ScaleMeasurement>>> = Arc::new(RwLock::new(None));
    static ref TELEGRAM_USERIDS: Arc<RwLock<HashSet<UserId>>> =
        Arc::new(RwLock::new(HashSet::new()));
}

pub fn run_bot(telegram_bot_token: &str, pool: PgPool, scope: &Scope) -> Result<(), Error> {
    let (s, r) = unbounded();

    let pool_ = pool.clone();
    let userid_handle = scope.spawn(move |_| fill_telegram_user_ids(pool_));
    let message_handle = scope.spawn(move |_| process_messages(r, pool));

    let mut core = Core::new()?;

    let api = Api::configure(telegram_bot_token)
        .build(core.handle())
        .map_err(|e| format_err!("{}", e))?;

    // Fetch new updates via long poll method
    let future = api.stream().for_each(|update| {
        // If the received update contains a new message...
        if let UpdateKind::Message(message) = update.kind {
            if let MessageKind::Text { ref data, .. } = message.kind {
                // Print received text message to stdout.
                debug!("{:?}", message);
                if TELEGRAM_USERIDS.read().contains(&message.from.id) {
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
                            Ok(meas) => match s.try_send(meas) {
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

        Ok(())
    });

    core.run(future).map_err(|e| format_err!("{}", e))?;
    drop(message_handle);
    drop(userid_handle);
    Ok(())
}

fn process_messages(r: Receiver<ScaleMeasurement>, pool: PgPool) {
    if let Ok(meas_list) = ScaleMeasurement::read_from_db(&pool) {
        for meas in meas_list {
            let current_dt = meas.datetime;
            let last_meas = LAST_WEIGHT.write().replace(meas);
            if let Some(last) = last_meas {
                if last.datetime > current_dt {
                    LAST_WEIGHT.write().replace(last);
                }
            }
        }
    }
    debug!("LAST_WEIGHT {:?}", *LAST_WEIGHT.read());
    loop {
        if let Ok(meas) = r.recv() {
            if meas.insert_into_db(&pool).is_ok() {
                debug!("{:?}", meas);
                LAST_WEIGHT.write().replace(meas);
            }
        }
    }
}

fn fill_telegram_user_ids(pool: PgPool) {
    loop {
        if let Ok(telegram_userids) = list_of_telegram_user_ids(&pool) {
            let mut telegram_userid_set = TELEGRAM_USERIDS.write();
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
            let telegram_userid: i64 = row.get_idx(0)?;
            Ok(telegram_userid)
        })
        .collect()
}
