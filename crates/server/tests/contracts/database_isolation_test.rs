//! API server state must never cross a test-server boundary.
use crate::test_harness::TestServer;

#[tokio::test]
async fn postgres_servers_have_private_databases() {
    let first = TestServer::new().await;
    let second = TestServer::new().await;
    let first_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(first.db.pool())
        .await
        .unwrap();
    let second_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(second.db.pool())
        .await
        .unwrap();
    assert_ne!(first_name, second_name);
    sqlx::raw_sql(
        "CREATE TABLE isolation_probe (value int); INSERT INTO isolation_probe VALUES (1)",
    )
    .execute(first.db.pool())
    .await
    .unwrap();
    let absent: bool = sqlx::query_scalar("SELECT to_regclass('isolation_probe') IS NULL")
        .fetch_one(second.db.pool())
        .await
        .unwrap();
    assert!(absent);
}
