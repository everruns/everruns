//! Upgrade from the complete legacy schema, preserving identity and credential provenance.
use sqlx::Connection;
use uuid::Uuid;

#[tokio::test]
async fn cutover_preserves_chats_and_never_fans_out_credentials() {
    let mut connection = sqlx::PgConnection::connect(&crate::test_harness::get_database_url())
        .await
        .unwrap();
    // UUID-only schema names and checked-in migration bodies contain no caller input.
    let schema = format!("vu_upgrade_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema},public"
    )))
    .execute(&mut connection)
    .await
    .unwrap();
    let mut migrations = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect::<Vec<_>>();
    migrations.sort();
    let cutover = migrations
        .iter()
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("_virtual_users.sql")
        })
        .unwrap();
    for path in migrations.iter().take_while(|path| *path != cutover) {
        let sql = std::fs::read_to_string(path).unwrap();
        if sql.contains("-- no-transaction") {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&mut connection)
                .await
                .unwrap();
        } else {
            let mut tx = connection.begin().await.unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
    }
    let mut tx = connection.begin().await.unwrap();
    sqlx::raw_sql(include_str!("../fixtures/virtual_users_legacy.sql"))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        std::fs::read_to_string(cutover).unwrap().as_str(),
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../fixtures/virtual_users_assertions.sql"))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "SET search_path TO public; DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&mut connection)
    .await
    .unwrap();
}
