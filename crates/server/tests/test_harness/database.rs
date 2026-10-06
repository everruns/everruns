//! Databases for server tests: the shared Postgres-mode database and the
//! private per-test copies on the process's embedded PostgreSQL.

use sqlx::{PgPool, postgres::PgPoolOptions};

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
                .block_on(async {
                    let pg = embedded().await;
                    pg.create_database("everruns_test", Some(migrated_template(pg).await))
                        .await
                        .expect("create shared test database");
                    pg.url("everruns_test")
                })
        })
        .join()
        .expect("start shared test database")
    })
    .clone()
}

async fn embedded() -> &'static everruns_pg_embedded::EmbeddedPostgres {
    everruns_pg_embedded::EmbeddedPostgres::shared()
        .await
        .expect("start embedded PostgreSQL")
}

/// Name of a database holding the migrated schema, created once per process.
/// Copies of it are how tests get a schema without running migrations again.
async fn migrated_template(pg: &everruns_pg_embedded::EmbeddedPostgres) -> &'static str {
    static TEMPLATE: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    const TEMPLATE_DB: &str = "everruns_template";
    TEMPLATE
        .get_or_init(|| async {
            pg.create_database(TEMPLATE_DB, None)
                .await
                .expect("create template database");
            let pool = PgPool::connect(&pg.url(TEMPLATE_DB))
                .await
                .expect("connect to template database");
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .expect("migrate template database");
            // A template cannot be copied while anything is connected to it.
            pool.close().await;
        })
        .await;
    TEMPLATE_DB
}

/// Create a PostgreSQL pool for tests
pub async fn create_test_pool() -> PgPool {
    let database_url = get_database_url();
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.")
}

/// A database of one test on the embedded cluster, dropped with its owner.
///
/// Each copy of the migrated schema is about 20 MB, and a test binary creates
/// hundreds; keeping them until the process exits fills the disk.
pub struct IsolatedDatabase {
    pub pool: PgPool,
    name: String,
    url: String,
}

impl IsolatedDatabase {
    /// Connection URL, for tests that open their own connections.
    pub fn url(&self) -> String {
        self.url.clone()
    }
}

impl Drop for IsolatedDatabase {
    fn drop(&mut self) {
        let name = std::mem::take(&mut self.name);
        // Drop runs inside the test's runtime, which cannot block on its own
        // futures, so a short-lived runtime on its own thread drops the
        // database. Never join it: the pool may have a connection half way
        // through authentication on the test's runtime, and DROP DATABASE
        // waits for that backend until the runtime closes its socket (or
        // authentication_timeout, 60s, passes).
        std::thread::spawn(move || {
            let dropped = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| {
                    runtime.block_on(async {
                        everruns_pg_embedded::EmbeddedPostgres::shared()
                            .await?
                            .drop_database(&name)
                            .await
                    })
                });
            if let Err(error) = dropped {
                eprintln!("could not drop test database: {error:#}");
            }
        });
    }
}

/// A fresh, migrated database of its own on this process's embedded
/// PostgreSQL, for tests that must not see each other's rows.
///
/// Migrations run once per test binary into a template database; every call
/// then copies that template, which takes milliseconds.
pub async fn isolated_test_database() -> IsolatedDatabase {
    let pg = embedded().await;
    let template = migrated_template(pg).await;
    let name = format!("test_{}", uuid::Uuid::now_v7().simple());
    pg.create_database(&name, Some(template))
        .await
        .expect("copy template database");
    // Every test gets its own pool on one shared cluster, so keep each small.
    let url = pg.url(&name);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to test database");
    IsolatedDatabase { pool, name, url }
}
