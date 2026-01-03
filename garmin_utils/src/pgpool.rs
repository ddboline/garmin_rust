use deadpool_postgres::{Client, Config, Pool};
use stack_string::StackString;
use std::{fmt, sync::Arc};
use tokio_postgres::{Config as PgConfig, NoTls};

pub use tokio_postgres::Transaction as PgTransaction;

use garmin_lib::errors::GarminError as Error;

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
    /// # Errors
    /// Return error if pool setup fails
    pub fn new(pgurl: &str) -> Result<Self, Error> {
        let pgconf: PgConfig = pgurl.parse()?;

        let mut config = Config::default();

        if let tokio_postgres::config::Host::Tcp(s) = &pgconf.get_hosts()[0] {
            config.host.replace(s.clone());
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

        let pool = config.builder(NoTls)?.max_size(4).build()?;

        Ok(Self {
            pgurl: Arc::new(pgurl.into()),
            pool: Some(pool),
        })
    }

    /// # Errors
    /// Return error if pool doesn't exist or we cannot pull connection from
    /// pool
    pub async fn get(&self) -> Result<Client, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| Error::StaticCustomError("No Pool Exists"))?
            .get()
            .await
            .map_err(Into::into)
    }
}
