//! Throwaway databases for tests, on this process's embedded PostgreSQL.
//!
//! Decision: tests run against the same repositories as production instead of
//! a hand-written in-memory copy of them. The in-memory backend drifted from
//! the SQL (no foreign keys, no unique constraints, its own ordering rules),
//! so a test could pass on behavior production never had. Each test gets a
//! private copy of a migrated template database, which takes milliseconds, and
//! the copy is dropped when the last clone of its backend goes away.

use std::sync::Arc;

use sqlx::{PgPool, postgres::PgPoolOptions};

use super::StorageBackend;
use super::repositories::Database;

/// Name of the template every test database is copied from.
const TEMPLATE_DB: &str = "everruns_template";

/// Owns one test database and drops it with the last backend that uses it.
///
/// Each copy of the migrated schema is about 20 MB, and a test binary creates
/// hundreds; keeping them until the process exits fills the disk.
pub struct TestDatabase {
    name: String,
    url: String,
    hooks: TestHooks,
}

/// Instrumentation tests attach to a backend: counters and fault injection.
#[derive(Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct TestHooks {
    session_list_lookups: std::sync::atomic::AtomicUsize,
    session_list_lookup_delay_ms: std::sync::atomic::AtomicU64,
    forced_failures: parking_lot::Mutex<Vec<String>>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl TestHooks {
    pub(crate) fn record_session_list_lookup(&self) -> u64 {
        use std::sync::atomic::Ordering::Relaxed;
        self.session_list_lookups.fetch_add(1, Relaxed);
        self.session_list_lookup_delay_ms.load(Relaxed)
    }

    pub(crate) fn reset_session_list_lookup_count(&self) {
        self.session_list_lookups
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn session_list_lookup_count(&self) -> usize {
        self.session_list_lookups
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn set_session_list_lookup_delay_ms(&self, delay_ms: u64) {
        self.session_list_lookup_delay_ms
            .store(delay_ms, std::sync::atomic::Ordering::Relaxed);
    }

    /// Queue a one-shot failure for `method`.
    pub(crate) fn force_failure(&self, method: &str) {
        self.forced_failures.lock().push(method.to_string());
    }

    /// Consume a queued failure for `method`, if one was requested.
    pub(crate) fn take_forced_failure(&self, method: &str) -> bool {
        let mut forced = self.forced_failures.lock();
        match forced.iter().position(|entry| entry == method) {
            Some(index) => {
                forced.remove(index);
                true
            }
            None => false,
        }
    }
}

impl TestDatabase {
    /// Connection URL, for tests that open their own connections.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Counters and fault injection shared by every clone of the backend.
    pub fn hooks(&self) -> &TestHooks {
        &self.hooks
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let name = std::mem::take(&mut self.name);
        // Drop runs inside the test's runtime, which cannot block on its own
        // futures, so a short-lived runtime on its own thread drops the
        // database. Never join it: a pool may have a connection half way
        // through authentication on the test's runtime, and DROP DATABASE
        // waits for that backend until the runtime closes its socket (or
        // authentication_timeout, 60s, passes).
        std::thread::spawn(move || {
            let dropped = run_on_own_runtime(async move {
                everruns_pg_embedded::EmbeddedPostgres::shared()
                    .await?
                    .drop_database(&name)
                    .await
            });
            if let Err(error) = dropped {
                eprintln!("could not drop test database: {error:#}");
            }
        });
    }
}

/// Run a future to completion on a fresh runtime on the current thread.
fn run_on_own_runtime<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

/// Create database `name` holding the migrated schema, and return its URL.
///
/// Migrations run once per process into a template database; every call then
/// copies that template, which takes milliseconds. The caller owns dropping it.
pub async fn create_migrated_database(name: &str) -> anyhow::Result<String> {
    let pg = everruns_pg_embedded::EmbeddedPostgres::shared().await?;
    pg.create_database(name, Some(migrated_template().await?))
        .await?;
    Ok(pg.url(name))
}

/// The template holding the migrated schema, created once per process.
async fn migrated_template() -> anyhow::Result<&'static str> {
    static TEMPLATE: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    TEMPLATE
        .get_or_try_init(|| async {
            let pg = everruns_pg_embedded::EmbeddedPostgres::shared().await?;
            pg.create_database(TEMPLATE_DB, None).await?;
            let pool = PgPool::connect(&pg.url(TEMPLATE_DB)).await?;
            sqlx::migrate!("./migrations").run(&pool).await?;
            // A template cannot be copied while anything is connected to it.
            pool.close().await;
            anyhow::Ok(())
        })
        .await?;
    Ok(TEMPLATE_DB)
}

/// The migrated template plus [`FIXTURES`], created once per process.
async fn fixture_template() -> anyhow::Result<&'static str> {
    static TEMPLATE: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    TEMPLATE
        .get_or_try_init(|| async {
            let pg = everruns_pg_embedded::EmbeddedPostgres::shared().await?;
            pg.create_database(FIXTURE_TEMPLATE_DB, Some(migrated_template().await?))
                .await?;
            let pool = PgPool::connect(&pg.url(FIXTURE_TEMPLATE_DB)).await?;
            sqlx::raw_sql(FIXTURES).execute(&pool).await?;
            pool.close().await;
            anyhow::Ok(())
        })
        .await?;
    Ok(FIXTURE_TEMPLATE_DB)
}

/// Template of [`create_test_database`]: the schema plus [`FIXTURES`].
const FIXTURE_TEMPLATE_DB: &str = "everruns_template_fixtures";

/// Rows unit tests may reference without creating them.
///
/// Unit tests name orgs by small literal ids, well-known principals by
/// `PrincipalId::from_seed(n)`, and harnesses and agents by `from_seed(n)`
/// (the nil id among them), and the schema holds all of them to existing
/// rows. Principals live in the default org. Harnesses and agents live in an
/// org of their own, so they never count toward the default org's limits or
/// show up in its listings. The org sequence moves past the fixtures so orgs a
/// test creates never collide with them.
const FIXTURES: &str = "
INSERT INTO organizations (org_id, public_id, name)
SELECT n, 'org_' || lpad(to_hex(n), 32, '0'), 'Test org ' || n
FROM unnest(ARRAY[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 20, 30, 41, 42, 43, 50, 64, 99, 100, 999]) AS n;
SELECT setval('organizations_org_id_seq', 1000);
INSERT INTO principals (id, public_id, org_id, kind)
SELECT id, 'principal_' || replace(id::text, '-', ''), 1, 'system'
FROM (SELECT ('00000000-0000-0000-0000-' || lpad(to_hex(n), 12, '0'))::uuid AS id
      FROM unnest(ARRAY[1, 2, 3, 30]) AS n) AS seeds;
INSERT INTO harnesses (id, org_id, name, system_prompt)
SELECT ('00000000-0000-0000-0000-' || lpad(to_hex(n), 12, '0'))::uuid, 999,
       'Test fixture harness ' || n, ''
FROM unnest(ARRAY[0, 1, 2]) AS n;
INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
SELECT ('00000000-0000-0000-0000-' || lpad(to_hex(n), 12, '0'))::uuid, 999,
       'agent_' || lpad(to_hex(n), 32, 'f'), 'Test fixture agent ' || n, '',
       '00000000-0000-0000-0000-000000000000'
FROM unnest(ARRAY[0, 1, 2]) AS n;
";

/// A fresh, migrated database of its own on the embedded cluster.
///
/// Sync so tests can build one anywhere: the database is created on a helper
/// thread with its own runtime, and the pool connects lazily on first use.
pub fn create_test_database() -> (PgPool, Arc<TestDatabase>) {
    let name = format!("test_{}", uuid::Uuid::now_v7().simple());
    let url = {
        let name = name.clone();
        std::thread::spawn(move || {
            run_on_own_runtime(async move {
                let pg = everruns_pg_embedded::EmbeddedPostgres::shared().await?;
                pg.create_database(&name, Some(fixture_template().await?))
                    .await?;
                Ok(pg.url(&name))
            })
        })
        .join()
        .expect("test database thread panicked")
        .expect("create test database")
    };
    // A lazy pool spawns its maintenance task right away, which needs a
    // runtime; plain `#[test]`s have none, so lend them a process-wide one.
    static RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build test database runtime")
    });
    let _runtime = tokio::runtime::Handle::try_current()
        .is_err()
        .then(|| RUNTIME.enter());
    // Every test gets its own pool on one shared cluster, so keep each small.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_lazy(&url)
        .expect("valid test database URL");
    (
        pool,
        Arc::new(TestDatabase {
            name,
            url,
            hooks: TestHooks::default(),
        }),
    )
}

/// A database of its own on the embedded cluster, migrated up to but not
/// including `version`, for tests that convert pre-migration data with that
/// migration. Slower than [`create_test_database`]: it runs the migrations
/// instead of copying a template.
pub async fn create_database_migrated_before(
    version: i64,
) -> anyhow::Result<(PgPool, Arc<TestDatabase>)> {
    let pg = everruns_pg_embedded::EmbeddedPostgres::shared().await?;
    let name = format!("test_{}", uuid::Uuid::now_v7().simple());
    pg.create_database(&name, None).await?;
    let url = pg.url(&name);
    let database = Arc::new(TestDatabase {
        name,
        url: url.clone(),
        hooks: TestHooks::default(),
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.migrations = migrator
        .migrations
        .iter()
        .filter(|migration| migration.version < version)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    migrator.run(&pool).await?;
    Ok((pool, database))
}

impl StorageBackend {
    /// A backend on a private, migrated database, for tests.
    ///
    /// The database is dropped when the last clone of this backend is.
    pub fn test_database() -> Self {
        let (pool, database) = create_test_database();
        Self::from_database(Database::new(pool).with_test_database(database))
    }

    /// The database a [`StorageBackend::test_database`] backend runs on.
    pub fn test_database_handle(&self) -> Option<&Arc<TestDatabase>> {
        self.database().test_database()
    }
}

/// A session row with nothing set beyond its org and the fixture owner.
pub fn test_session_row(org_id: i64) -> super::CreateSessionRow {
    super::CreateSessionRow {
        playground_user_id: None,
        source: crate::domains::sessions::record::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id: None,
        agent_revision: None,
        virtual_user_id: None,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    }
}

impl StorageBackend {
    /// Create a session in the default org, for tests that only need one to
    /// exist (files, tasks and schedules reference their session).
    pub async fn create_test_session(&self) -> everruns_contracts::typed_id::SessionId {
        self.create_session(test_session_row(everruns_core::DEFAULT_ORG_ID))
            .await
            .expect("create test session")
            .id
    }

    /// Create a user with `id`, for tests that reference one by id. A user
    /// that already exists is left as it is.
    pub async fn create_test_user(&self, id: uuid::Uuid) -> uuid::Uuid {
        self.create_user_with_id(
            id,
            super::CreateUserRow {
                email: format!("{}@test.invalid", id.simple()),
                name: "Test user".to_string(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            },
        )
        .await
        .expect("create test user");
        id
    }

    /// [`Self::create_test_user`] for each id, e.g. an optional owner.
    pub async fn create_test_users(&self, ids: impl IntoIterator<Item = uuid::Uuid>) {
        for id in ids {
            self.create_test_user(id).await;
        }
    }

    /// Insert a bare harness with `id` into `org_id`, for tests that name one
    /// by id.
    pub async fn create_test_harness(&self, org_id: i64, id: uuid::Uuid) {
        sqlx::query(
            "INSERT INTO harnesses (id, org_id, name, system_prompt) VALUES ($1, $2, $3, '')",
        )
        .bind(id)
        .bind(org_id)
        .bind(format!("Test harness {}", id.simple()))
        .execute(self.database().pool())
        .await
        .expect("create test harness");
    }
    /// Insert a system principal with `id` into the default org, for tests
    /// that own a session or row by a principal id. Existing ids are kept.
    pub async fn create_test_principal(
        &self,
        id: everruns_contracts::typed_id::PrincipalId,
    ) -> everruns_contracts::typed_id::PrincipalId {
        sqlx::query(
            "INSERT INTO principals (id, public_id, org_id, kind) \
             VALUES ($1, 'principal_' || replace($1::text, '-', ''), 1, 'system') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id.uuid())
        .execute(self.database().pool())
        .await
        .expect("create test principal");
        id
    }

    /// Insert a bare agent with `id` into `org_id` on the fixture harness, for
    /// tests that name one by id. Returns `id`.
    pub async fn create_test_agent(&self, org_id: i64, id: uuid::Uuid) -> uuid::Uuid {
        sqlx::query(
            "INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id) \
             VALUES ($1, $2, 'agent_' || replace($1::text, '-', ''), $3, '', $4)",
        )
        .bind(id)
        .bind(org_id)
        .bind(format!("Test agent {}", id.simple()))
        .bind(uuid::Uuid::nil())
        .execute(self.database().pool())
        .await
        .expect("create test agent");
        id
    }

    /// Create a Slack app in `org_id` on the fixture harness and agent, for
    /// tests that only need an app row to point at. Returns its internal id.
    pub async fn create_test_app(&self, org_id: i64) -> uuid::Uuid {
        self.create_app(
            org_id,
            super::CreateAppRow {
                public_id: format!("app_{}", uuid::Uuid::now_v7().simple()),
                name: "Test app".to_string(),
                description: None,
                harness_id: uuid::Uuid::nil(),
                agent_id: Some(uuid::Uuid::nil()),
                virtual_user_id: None,
                owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                channel_type: Some("slack".to_string()),
                channel_config: serde_json::json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .expect("create test app")
        .id
    }

    /// Insert a service virtual user into `org_id`, for tests that name one.
    pub async fn create_test_virtual_user(
        &self,
        org_id: i64,
    ) -> everruns_contracts::typed_id::VirtualUserId {
        self.create_test_virtual_user_with_id(org_id, uuid::Uuid::now_v7())
            .await
    }

    /// Insert a service virtual user with `id` into `org_id`. Connections and
    /// pins are keyed by virtual user, so a test that keys them by a user's
    /// id gives that user a virtual user of the same id.
    pub async fn create_test_virtual_user_with_id(
        &self,
        org_id: i64,
        id: uuid::Uuid,
    ) -> everruns_contracts::typed_id::VirtualUserId {
        sqlx::query("INSERT INTO virtual_users (id, org_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(org_id)
            .bind(format!("Test virtual user {}", id.simple()))
            .execute(self.database().pool())
            .await
            .expect("create test virtual user");
        everruns_contracts::typed_id::VirtualUserId::from_uuid(id)
    }

    /// Insert a bare MCP server with `id` into `org_id`, for tests that name
    /// one by id. Returns `id`.
    pub async fn create_test_mcp_server(&self, org_id: i64, id: uuid::Uuid) -> uuid::Uuid {
        sqlx::query("INSERT INTO mcp_servers (id, org_id, name, url) VALUES ($1, $2, $3, 'https://mcp.test.invalid/mcp')")
            .bind(id)
            .bind(org_id)
            .bind(format!("Test MCP server {}", id.simple()))
            .execute(self.database().pool())
            .await
            .expect("create test MCP server");
        id
    }

    /// A connection with foreign keys and triggers switched off, for tests
    /// that need a row production can no longer write: a dangling parent, an
    /// id from a retired level, a back-dated `updated_at`. Detached from the
    /// pool, so the setting never leaks into another query.
    pub async fn unchecked_connection(&self) -> sqlx::PgConnection {
        let mut conn = self
            .database()
            .pool()
            .acquire()
            .await
            .expect("acquire test connection")
            .detach();
        sqlx::query("SET session_replication_role = replica")
            .execute(&mut conn)
            .await
            .expect("disable constraints");
        conn
    }
}
