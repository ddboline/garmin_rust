use crossbeam_channel::{unbounded, Receiver};
use crossbeam_utils::thread::Scope;
use failure::{err_msg, Error};
use futures::Stream;
use log::debug;
use parking_lot::RwLock;
use std::sync::Arc;
use telegram_bot::types::refs::UserId;
use telegram_bot::{Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio_core::reactor::Core;

use crate::scale_measurement::ScaleMeasurement;
use garmin_lib::common::pgpool::PgPool;

lazy_static! {
    static ref LAST_WEIGHT: Arc<RwLock<Option<ScaleMeasurement>>> = Arc::new(RwLock::new(None));
}

pub fn run_bot(telegram_bot_token: &str, pool: PgPool, scope: &Scope) -> Result<(), Error> {
    let (s, r) = unbounded();

    let handle = scope.spawn(move |_| process_messages(r.clone(), pool.clone()));

    let mut core = Core::new()?;

    let api = Api::configure(telegram_bot_token)
        .build(core.handle())
        .map_err(|e| err_msg(format!("{}", e)))?;

    // Fetch new updates via long poll method
    let future = api.stream().for_each(|update| {
        // If the received update contains a new message...
        if let UpdateKind::Message(message) = update.kind {
            if let MessageKind::Text { ref data, .. } = message.kind {
                // Print received text message to stdout.
                debug!("{:?}", message);
                if message.from.id == UserId::new(972_549_683) {
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
                        "Hi, {}! You just wrote '{}'",
                        &message.from.first_name, data
                    )));
                }
            }
        }

        Ok(())
    });

    core.run(future).map_err(|e| err_msg(format!("{}", e)))?;
    drop(handle);
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
