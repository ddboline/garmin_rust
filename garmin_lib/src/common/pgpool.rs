use anyhow::Error;
use deadpool::managed::Object;
use deadpool_postgres::{ClientWrapper, Config, Pool};
use std::env::var;
use std::fmt;
use tokio_postgres::error::Error as PgError;
use tokio_postgres::NoTls;

/// Wrapper around `r2d2::Pool`, two pools are considered equal if they have the same connection string
/// The only way to use `PgPool` is through the get method, which returns a `PooledConnection` object
#[derive(Clone)]
pub struct PgPool {
    pgurl: String,
    pool: Pool,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PgPool {}", self.pgurl)
    }
}

impl Default for PgPool {
    fn default() -> Self {
        let pgurl = var("PGURL").expect("PGURL NOT SET!");
        let config = Config::from_env("PGURL").expect("Failed to create config");
        Self {
            pgurl: pgurl.to_string(),
            pool: config.create_pool(NoTls).expect("Failed to create pool"),
        }
    }
}

impl PartialEq for PgPool {
    fn eq(&self, other: &Self) -> bool {
        self.pgurl == other.pgurl
    }
}

impl PgPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self) -> Result<Object<ClientWrapper, PgError>, Error> {
        self.pool.get().await.map_err(Into::into)
    }
}
