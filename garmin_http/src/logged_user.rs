use anyhow::Error;
pub use auth_server_rust::logged_user::{
    LoggedUser, AUTHORIZED_USERS, JWT_SECRET, SECRET_KEY, TRIGGER_DB_UPDATE,
};
use log::debug;
use stack_string::StackString;
use std::env::var;

use garmin_lib::common::pgpool::PgPool;

pub async fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    debug!("{:?}", *TRIGGER_DB_UPDATE);
    let users = if TRIGGER_DB_UPDATE.check() {
        let query = "SELECT email FROM authorized_users";
        let results: Result<Vec<_>, Error> = pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .into_iter()
            .map(|row| {
                let email: StackString = row.try_get(0)?;
                Ok(LoggedUser { email })
            })
            .collect();
        results?
    } else {
        AUTHORIZED_USERS.get_users()
    };
    if let Ok("true") = var("TESTENV").as_ref().map(String::as_str) {
        let user = LoggedUser {
            email: "user@test".into(),
        };
        AUTHORIZED_USERS.merge_users(&[user])?;
    }
    AUTHORIZED_USERS.merge_users(&users)?;
    debug!("{:?}", *AUTHORIZED_USERS);
    Ok(())
}
