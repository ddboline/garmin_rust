use crossbeam_channel::{unbounded, Receiver};
use failure::{err_msg, Error};
use futures::Stream;
use std::thread;
use telegram_bot::types::refs::UserId;
use telegram_bot::{Api, CanReplySendMessage, MessageKind, UpdateKind};
use tokio_core::reactor::Core;

use crate::scale_measurement::ScaleMeasurement;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

pub fn run_bot(config: &GarminConfig, pool: PgPool) -> Result<(), Error> {
    let (s, r) = unbounded();

    thread::spawn(move || process_messages(r.clone(), pool.clone()));

    let mut core = Core::new()?;

    let api = Api::configure(&config.telegram_bot_token)
        .build(core.handle())
        .map_err(|e| err_msg(format!("{}", e)))?;

    // Fetch new updates via long poll method
    let future = api.stream().for_each(|update| {
        // If the received update contains a new message...
        if let UpdateKind::Message(message) = update.kind {
            if let MessageKind::Text { ref data, .. } = message.kind {
                // Print received text message to stdout.
                println!("{:?}", message);
                if message.from.id == UserId::new(972_549_683) {
                    if let Err(e) = s.try_send(data.to_string()) {
                        println!("send error {}", e);
                    }
                }

                // Answer message with "Hi".
                api.spawn(message.text_reply(format!(
                    "Hi, {}! You just wrote '{}'",
                    &message.from.first_name, data
                )));
            }
        }

        Ok(())
    });

    core.run(future).map_err(|e| err_msg(format!("{}", e)))?;
    Ok(())
}

fn process_messages(r: Receiver<String>, pool: PgPool) {
    loop {
        if let Ok(msg) = r.recv() {
            if let Ok(meas) = ScaleMeasurement::from_telegram_text(&msg) {
                if meas.insert_into_db(&pool).is_ok() {
                    println!("{:?}", meas);
                }
            }
        }
    }
}
