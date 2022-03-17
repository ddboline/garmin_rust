use anyhow::{format_err, Error};
use deadpool_postgres::{Client, Config, Pool};
use std::{fmt, sync::Arc};
use tokio_postgres::{Config as PgConfig, NoTls};

pub use tokio_postgres::Transaction as PgTransaction;

use stack_string::StackString;

#[derive(Clone, Default)]
pub struct PgPool {
    pgurl: Arc<StackString>,
    pool: Option<Pool>,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PgPool {}", &self.pgurl)
    }
}

impl PgPool {
    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn new(pgurl: &str) -> Self {
        let pgconf: PgConfig = pgurl.parse().expect("Failed to parse Url");

        let mut config = Config::default();

        if let tokio_postgres::config::Host::Tcp(s) = &pgconf.get_hosts()[0] {
            config.host.replace(s.to_string());
        }
        if let Some(u) = pgconf.get_user() {
            config.user.replace(u.to_string());
        }
        if let Some(p) = pgconf.get_password() {
            config
                .password
                .replace(String::from_utf8_lossy(p).to_string());
        }
        if let Some(db) = pgconf.get_dbname() {
            config.dbname.replace(db.to_string());
        }

        Self {
            pgurl: Arc::new(pgurl.into()),
            pool: Some(
                config
                    .create_pool(None, NoTls)
                    .unwrap_or_else(|_| panic!("Failed to create pool {}", pgurl)),
            ),
        }
    }

    /// # Errors
    /// Return error if pool doesn't exist or we cannot pull connection from
    /// pool
    pub async fn get(&self) -> Result<Client, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| format_err!("No Pool Exists"))?
            .get()
            .await
            .map_err(Into::into)
    }
}
