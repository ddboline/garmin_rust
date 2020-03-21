use anyhow::{format_err, Error};
use deadpool::managed::Object;
use deadpool_postgres::{ClientWrapper, Config, Pool};
use std::fmt;
use tokio_postgres::{error::Error as PgError, Config as PgConfig, NoTls};

/// Wrapper around `r2d2::Pool`, two pools are considered equal if they have the
/// same connection string The only way to use `PgPool` is through the get
/// method, which returns a `PooledConnection` object
#[derive(Clone, Default)]
pub struct PgPool {
    pgurl: String,
    pool: Option<Pool>,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PgPool {}", self.pgurl)
    }
}

impl PartialEq for PgPool {
    fn eq(&self, other: &Self) -> bool {
        self.pgurl == other.pgurl
    }
}

impl PgPool {
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
            pgurl: pgurl.to_string(),
            pool: Some(
                config
                    .create_pool(NoTls)
                    .unwrap_or_else(|_| panic!("Failed to create pool {}", pgurl)),
            ),
        }
    }

    pub async fn get(&self) -> Result<Object<ClientWrapper, PgError>, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| format_err!("No Pool Exists"))?
            .get()
            .await
            .map_err(Into::into)
    }
}
