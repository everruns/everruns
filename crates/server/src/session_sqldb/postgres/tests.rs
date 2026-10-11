//! The persisted store against a real database.

use super::*;
use crate::storage::StorageBackend;
use crate::storage::test_database::test_session_row;
use serde_json::json;

const ORG: i64 = 2;

struct Fixture {
    backend: StorageBackend,
    session: SessionId,
}

impl Fixture {
    async fn new() -> Self {
        let backend = StorageBackend::test_database();
        let session = backend
            .create_session(test_session_row(ORG))
            .await
            .expect("create session")
            .id;
        Self { backend, session }
    }

    /// A store as another server process would hold it: same rows, own cache.
    fn store(&self) -> PostgresSqlDbStore {
        PostgresSqlDbStore::new(self.backend.database().clone())
    }

    async fn row(&self, name: &str) -> SessionDatabaseRow {
        self.backend
            .get_session_database(self.session, name)
            .await
            .unwrap()
            .expect("database row")
    }

    /// `(page_number, xmin)` of every stored page: a rewritten page gets a
    /// new `xmin`, an untouched one keeps it.
    async fn page_versions(&self, name: &str) -> Vec<(i32, String)> {
        let id = self.row(name).await.id;
        sqlx::query_as(
            "SELECT page_number, xmin::TEXT FROM session_database_pages \
             WHERE database_id = $1 ORDER BY page_number",
        )
        .bind(id)
        .fetch_all(self.backend.database().pool())
        .await
        .unwrap()
    }

    async fn page_rows(&self, database_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM session_database_pages WHERE database_id = $1")
            .bind(database_id)
            .fetch_one(self.backend.database().pool())
            .await
            .unwrap()
    }
}

async fn count(store: &PostgresSqlDbStore, session: SessionId, sql: &str) -> serde_json::Value {
    store.sql_query(session, "db", sql).await.unwrap().rows[0][0].clone()
}

#[tokio::test]
async fn write_is_visible_to_a_fresh_store() {
    let fx = Fixture::new().await;
    let writer = fx.store();
    writer
        .sql_execute(
            fx.session,
            "db",
            "CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('kept')",
        )
        .await
        .unwrap();

    let row = fx.row("db").await;
    assert!(row.page_count > 0);
    assert_eq!(row.size_bytes, i64::from(row.page_count) * 4096);
    assert_eq!(fx.page_rows(row.id).await, i64::from(row.page_count));

    let restarted = fx.store();
    let result = restarted
        .sql_query(fx.session, "db", "SELECT v FROM t")
        .await
        .unwrap();
    assert_eq!(result.rows, vec![vec![json!("kept")]]);
    let schema = restarted
        .sql_schema(fx.session, "db", Some("t"))
        .await
        .unwrap();
    assert_eq!(schema[0].row_count, 1);
    let info = restarted
        .get_database(fx.session, "db")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.page_count, row.page_count);
}

#[tokio::test]
async fn small_write_rewrites_only_changed_pages() {
    let fx = Fixture::new().await;
    let store = fx.store();
    store
        .sql_execute(
            fx.session,
            "db",
            "CREATE TABLE big (v TEXT); CREATE TABLE small (v TEXT);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 400)
             INSERT INTO big SELECT hex(randomblob(100)) FROM n;",
        )
        .await
        .unwrap();
    let before = fx.page_versions("db").await;
    assert!(before.len() > 20, "{} pages", before.len());

    store
        .sql_execute(fx.session, "db", "INSERT INTO small VALUES ('x')")
        .await
        .unwrap();
    let after = fx.page_versions("db").await;
    assert_eq!(after.len(), before.len());
    let rewritten = before
        .iter()
        .zip(&after)
        .filter(|(old, new)| old.1 != new.1)
        .count();
    assert!(rewritten > 0);
    assert!(
        rewritten <= 4,
        "{rewritten} of {} pages rewritten for a one-row insert",
        before.len()
    );
}

#[tokio::test]
async fn reads_write_nothing() {
    let fx = Fixture::new().await;
    let store = fx.store();
    store
        .sql_execute(
            fx.session,
            "db",
            "CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (1)",
        )
        .await
        .unwrap();
    let generation = fx.row("db").await.generation;
    let pages = fx.page_versions("db").await;

    store
        .sql_query(fx.session, "db", "SELECT * FROM t")
        .await
        .unwrap();
    store.sql_schema(fx.session, "db", None).await.unwrap();
    store
        .sql_execute(fx.session, "db", "SELECT COUNT(*) FROM t")
        .await
        .unwrap();
    let err = store
        .sql_query(fx.session, "db", "DELETE FROM t")
        .await
        .unwrap_err();
    assert!(matches!(err, SessionSqlDbError::QueryError(_)), "{err}");

    assert_eq!(fx.row("db").await.generation, generation);
    assert_eq!(fx.page_versions("db").await, pages);
    assert_eq!(
        count(&store, fx.session, "SELECT COUNT(*) FROM t").await,
        json!(1)
    );
}

#[tokio::test]
async fn stale_cache_reloads_instead_of_clobbering() {
    let fx = Fixture::new().await;
    let a = fx.store();
    let b = fx.store();
    a.sql_execute(fx.session, "db", "CREATE TABLE t (v TEXT)")
        .await
        .unwrap();
    // Both processes now hold a cache of the same generation.
    assert_eq!(
        count(&b, fx.session, "SELECT COUNT(*) FROM t").await,
        json!(0)
    );

    a.sql_execute(fx.session, "db", "INSERT INTO t VALUES ('from a')")
        .await
        .unwrap();
    b.sql_execute(fx.session, "db", "INSERT INTO t VALUES ('from b')")
        .await
        .unwrap();
    assert_eq!(
        count(&a, fx.session, "SELECT COUNT(*) FROM t").await,
        json!(2)
    );

    let result = fx
        .store()
        .sql_query(fx.session, "db", "SELECT v FROM t ORDER BY v")
        .await
        .unwrap();
    assert_eq!(
        result.rows,
        vec![vec![json!("from a")], vec![json!("from b")]]
    );
}

#[tokio::test]
async fn concurrent_writers_all_land() {
    let fx = Fixture::new().await;
    let stores: Vec<_> = (0..3).map(|_| Arc::new(fx.store())).collect();
    stores[0]
        .sql_execute(fx.session, "db", "CREATE TABLE t (v INTEGER)")
        .await
        .unwrap();
    let writes = stores.iter().enumerate().map(|(i, store)| {
        let store = store.clone();
        let session = fx.session;
        tokio::spawn(async move {
            store
                .sql_execute(session, "db", &format!("INSERT INTO t VALUES ({i})"))
                .await
        })
    });
    let mut landed = 0;
    for write in writes {
        // A write may give up after repeated conflicts; it must then leave
        // nothing behind rather than half a write.
        if write.await.unwrap().is_ok() {
            landed += 1;
        }
    }
    assert!(landed >= 1);
    assert_eq!(
        count(&fx.store(), fx.session, "SELECT COUNT(*) FROM t").await,
        json!(landed)
    );
}

#[tokio::test]
async fn stale_generation_commit_writes_nothing() {
    let fx = Fixture::new().await;
    fx.store()
        .sql_execute(fx.session, "db", "CREATE TABLE t (v INTEGER)")
        .await
        .unwrap();
    let row = fx.row("db").await;
    let pages = fx.page_versions("db").await;

    let committed = fx
        .backend
        .commit_session_database_pages(row.id, row.generation - 1, &[(1, vec![0; 4096])], 1, 4096)
        .await
        .unwrap();
    assert_eq!(committed, None);
    assert_eq!(fx.row("db").await.generation, row.generation);
    assert_eq!(fx.page_versions("db").await, pages);
}

#[tokio::test]
async fn failed_write_is_all_or_nothing() {
    let fx = Fixture::new().await;
    let store = fx.store();
    store
        .sql_execute(fx.session, "db", "CREATE TABLE t (v INTEGER)")
        .await
        .unwrap();
    let generation = fx.row("db").await.generation;

    let err = store
        .sql_execute(
            fx.session,
            "db",
            "INSERT INTO t VALUES (1); INSERT INTO missing VALUES (2)",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, SessionSqlDbError::QueryError(_)), "{err}");
    let err = store
        .sql_execute(fx.session, "db", "BEGIN; INSERT INTO t VALUES (3)")
        .await
        .unwrap_err();
    assert!(matches!(err, SessionSqlDbError::QueryError(_)), "{err}");

    assert_eq!(fx.row("db").await.generation, generation);
    assert_eq!(
        count(&store, fx.session, "SELECT COUNT(*) FROM t").await,
        json!(0)
    );
    store
        .sql_execute(fx.session, "db", "BEGIN; INSERT INTO t VALUES (4); COMMIT")
        .await
        .unwrap();
    assert_eq!(
        count(&fx.store(), fx.session, "SELECT COUNT(*) FROM t").await,
        json!(1)
    );
}

#[tokio::test]
async fn delete_removes_pages() {
    let fx = Fixture::new().await;
    let store = fx.store();
    store
        .sql_execute(fx.session, "db", "CREATE TABLE t (v INTEGER)")
        .await
        .unwrap();
    let id = fx.row("db").await.id;
    assert!(fx.page_rows(id).await > 0);

    assert!(store.delete_database(fx.session, "db").await.unwrap());
    assert!(!store.delete_database(fx.session, "db").await.unwrap());
    assert_eq!(fx.page_rows(id).await, 0);
    let err = store
        .sql_query(fx.session, "db", "SELECT 1")
        .await
        .unwrap_err();
    assert!(matches!(err, SessionSqlDbError::DatabaseNotFound(_)));

    // Recreating the name starts empty, even in the store that cached it.
    store
        .sql_execute(fx.session, "db", "CREATE TABLE t (v INTEGER)")
        .await
        .unwrap();

    // Deleting the session cascades to its databases and pages.
    let id = fx.row("db").await.id;
    assert!(fx.backend.delete_session(ORG, fx.session).await.unwrap());
    assert_eq!(fx.page_rows(id).await, 0);
    assert!(store.list_databases(fx.session).await.unwrap().is_empty());
}

#[tokio::test]
async fn create_enforces_name_and_count_limits() {
    let fx = Fixture::new().await;
    let store = fx.store();
    for i in 0..MAX_DATABASES_PER_SESSION {
        store
            .create_database(fx.session, &format!("db_{i}"))
            .await
            .unwrap();
    }
    assert!(matches!(
        store.create_database(fx.session, "db_0").await,
        Err(SessionSqlDbError::DatabaseAlreadyExists(_))
    ));
    assert!(matches!(
        store.create_database(fx.session, "one_more").await,
        Err(SessionSqlDbError::LimitExceeded(_))
    ));
    assert!(matches!(
        store
            .sql_execute(fx.session, "one_more", "CREATE TABLE t (v INTEGER)")
            .await,
        Err(SessionSqlDbError::LimitExceeded(_))
    ));
    assert!(matches!(
        store.create_database(fx.session, "bad-name").await,
        Err(SessionSqlDbError::InvalidDatabaseName(_))
    ));
    let names: Vec<_> = store
        .list_databases(fx.session)
        .await
        .unwrap()
        .into_iter()
        .map(|db| db.name)
        .collect();
    assert_eq!(names.len(), MAX_DATABASES_PER_SESSION);
}

#[tokio::test]
async fn fork_copies_databases_independently() {
    let fx = Fixture::new().await;
    let store = fx.store();
    store
        .sql_execute(
            fx.session,
            "db",
            "CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('parent')",
        )
        .await
        .unwrap();
    store.create_database(fx.session, "empty").await.unwrap();
    let child = fx
        .backend
        .create_session(test_session_row(ORG))
        .await
        .unwrap()
        .id;

    fx.backend
        .copy_session_databases(fx.session, child)
        .await
        .unwrap();

    let names: Vec<_> = store
        .list_databases(child)
        .await
        .unwrap()
        .into_iter()
        .map(|db| db.name)
        .collect();
    assert_eq!(names, vec!["db", "empty"]);
    store
        .sql_execute(child, "db", "INSERT INTO t VALUES ('child')")
        .await
        .unwrap();
    assert_eq!(
        count(&store, child, "SELECT COUNT(*) FROM t").await,
        json!(2)
    );
    assert_eq!(
        count(&store, fx.session, "SELECT COUNT(*) FROM t").await,
        json!(1)
    );
}
