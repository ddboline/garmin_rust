use actix::sync::SyncContext;
use actix::Actor;
use failure::{err_msg, Error};
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::{PostgresConnectionManager, TlsMode};
use std::fmt;

/// Wrapper around r2d2::Pool, two pools are considered equal if they have the same connection string
/// The only way to use PgPool is through the get method, which returns a PooledConnection object
#[derive(Clone)]
pub struct PgPool {
    pgurl: String,
    pool: Pool<PostgresConnectionManager>,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PgPool {}", self.pgurl)
    }
}

impl PartialEq for PgPool {
    fn eq(&self, other: &PgPool) -> bool {
        self.pgurl == other.pgurl
    }
}

impl PgPool {
    pub fn new(pgurl: &str) -> PgPool {
        let manager = PostgresConnectionManager::new(pgurl, TlsMode::None)
            .expect("Failed to open DB connection");
        PgPool {
            pgurl: pgurl.to_string(),
            pool: Pool::new(manager).expect("Failed to open DB connection"),
        }
    }

    pub fn get(&self) -> Result<PooledConnection<PostgresConnectionManager>, Error> {
        self.pool.get().map_err(err_msg)
    }
}

impl Actor for PgPool {
    type Context = SyncContext<Self>;
}
