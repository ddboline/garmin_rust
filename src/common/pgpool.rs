use failure::{err_msg, Error};
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::{PostgresConnectionManager, TlsMode};
use std::fmt;

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
