//! Private database fixtures and the shared raw-pool migration database.

use std::sync::Arc;

use everruns_server::storage::test_database::{
    TestDatabase, create_database_migrated_before, create_migrated_database,
    create_migrated_test_database, create_test_database,
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

/// Product-seeded API tests use a private migrated database in both modes.
pub async fn isolated_postgres_database() -> IsolatedDatabase {
    let (pool, database) = create_migrated_test_database();
    IsolatedDatabase { pool, database }
}

/// Prepare a private database before a single-test binary configures process
/// environment. Join the temporary runtime and close its pool first: creating a
/// lazy pool outside Tokio would leave a persistent maintenance runtime alive.
pub fn database_before_server_environment() -> Arc<TestDatabase> {
    std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build database fixture runtime")
            .block_on(async {
                let (pool, database) = create_migrated_test_database();
                pool.close().await;
                database
            })
    })
    .join()
    .expect("prepare database before configuring environment")
}

/// A database of its own on the embedded cluster, migrated up to but not
/// including `version`, for tests that convert pre-migration data with that
/// migration. Slower than [`isolated_test_database`]: it runs the migrations
/// instead of copying the template.
pub async fn database_migrated_before(version: i64) -> IsolatedDatabase {
    let (pool, database) = create_database_migrated_before(version)
        .await
        .expect("create partially migrated test database");
    IsolatedDatabase { pool, database }
}

/// Product seeding is fixture preparation, not part of every API assertion.
/// Keep a disconnected template per grade and fixture mode, then copy it for
/// each server. Every server still owns its rows, pools, counters, and cleanup;
/// tests of seeding itself continue to use fresh unseeded databases.
pub async fn seeded_database(
    mode: super::TestMode,
    grade: everruns_core::DeploymentGrade,
    host: &everruns_core::host::HostComposition,
    harnesses: &[everruns_server::domains::harnesses::record::BuiltInHarnessDefinition],
) -> IsolatedDatabase {
    use everruns_core::DeploymentGrade;
    use everruns_server::setup::seed::{SeedAuthContext, seed_all_with_host_composition};
    use tokio::sync::OnceCell;
    static TEMPLATES: [OnceCell<Arc<TestDatabase>>; 8] = [const { OnceCell::const_new() }; 8];
    let grade_index = match grade {
        DeploymentGrade::Dev => 0,
        DeploymentGrade::Poc => 1,
        DeploymentGrade::Preview => 2,
        DeploymentGrade::Prod => 3,
    };
    let mode_index = match mode {
        super::TestMode::InMemory => 0,
        super::TestMode::Postgres => 4,
    };
    let template = TEMPLATES[grade_index + mode_index]
        .get_or_init(|| async {
            let database = match mode {
                super::TestMode::InMemory => isolated_test_database().await,
                super::TestMode::Postgres => isolated_postgres_database().await,
            };
            seed_all_with_host_composition(
                &database.backend(),
                grade,
                &SeedAuthContext::default(),
                host,
                harnesses,
            )
            .await
            .expect("seed API fixture template");
            // PostgreSQL refuses a template with active connections. Await all
            // pool shutdown work before publishing it to other test runtimes.
            database.pool.close().await;
            database.database
        })
        .await;
    let (pool, database) = template.copy().await.expect("copy seeded API fixture");
    IsolatedDatabase { pool, database }
}
