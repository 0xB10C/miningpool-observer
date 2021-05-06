use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool, PoolError};

pub type PgPool = Pool<ConnectionManager<PgConnection>>;

pub fn new(database_url: &str) -> Result<PgPool, PoolError> {
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    Pool::builder().build(manager)
}
