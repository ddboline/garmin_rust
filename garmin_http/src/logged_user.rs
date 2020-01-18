use anyhow::Error;
pub use rust_auth_server::logged_user::{LoggedUser, AUTHORIZED_USERS};

use garmin_lib::common::pgpool::PgPool;

pub fn fill_from_db(pool: &PgPool) -> Result<(), Error> {
    let query = "SELECT email FROM authorized_users";
    let results: Result<Vec<_>, Error> = pool
        .get()?
        .query(query, &[])?
        .into_iter()
        .map(|row| {
            let email: String = row.try_get(0)?;
            Ok(LoggedUser { email })
        })
        .collect();
    let users = results?;

    AUTHORIZED_USERS.merge_users(&users)
}
