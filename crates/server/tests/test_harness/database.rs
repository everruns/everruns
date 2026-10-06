//! Databases for server tests: the shared Postgres-mode database and the
//! private per-test copies on the process's embedded PostgreSQL.

use std::sync::Arc;

use everruns_server::storage::test_database::{
    TestDatabase, create_migrated_database, create_test_database,
};
use everruns_server::storage::{Database, StorageBackend};
use sqlx::PgPool;

/// The database Postgres-mode tests share.
///
/// `DATABASE_URL` wins (CI and `just test` set it), then `DB_PORT` for a local
/// PostgreSQL. With neither, the tests of this process share one migrated
/// database on the embedded cluster, so a bare `cargo test` needs no server
/// and never sees rows a previous run left behind.
pub fn get_database_url() -> String {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return url;
    }
    if let Ok(port) = std::env::var("DB_PORT") {
        return format!("postgres://everruns:everruns@localhost:{port}/everruns_test");
    }
    static URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    URL.get_or_init(|| {
        // Callers are sync and often inside a test's runtime, which cannot
        // block on its own futures; a short-lived runtime on its own thread can.
        std::thread::spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime")
                .block_on(create_migrated_database("everruns_test"))
                .expect("create shared test database")
        })
        .join()
        .expect("start shared test database")
    })
    .clone()
}

/// Create a PostgreSQL pool for tests
pub async fn create_test_pool() -> PgPool {
    let database_url = get_database_url();
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.")
}

/// A database of one test on the embedded cluster, dropped with its owner.
pub struct IsolatedDatabase {
    pub pool: PgPool,
    database: Arc<TestDatabase>,
}

impl IsolatedDatabase {
    /// Connection URL, for tests that open their own connections.
    pub fn url(&self) -> String {
        self.database.url().to_string()
    }

    /// A storage backend on this database that keeps it alive.
    #[allow(dead_code)]
    pub fn backend(&self) -> StorageBackend {
        StorageBackend::from_database(
            Database::new(self.pool.clone()).with_test_database(self.database.clone()),
        )
    }
}

/// A fresh, migrated database of its own on this process's embedded
/// PostgreSQL, for tests that must not see each other's rows.
pub async fn isolated_test_database() -> IsolatedDatabase {
    let (pool, database) = create_test_database();
    IsolatedDatabase { pool, database }
}
