use crate::test_harness;

use sqlx::{Connection, Executor, PgConnection, Row};

#[tokio::test]
async fn narrow_personal_access_tokens_are_deleted_and_full_ones_kept() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut transaction = connection.begin().await.expect("begin transaction");
    transaction
        .execute("SET LOCAL search_path TO pg_temp, public")
        .await
        .expect("set isolated migration search path");
    sqlx::raw_sql(
        r#"
        CREATE TEMPORARY TABLE personal_access_tokens (
            name TEXT PRIMARY KEY,
            scopes JSONB NOT NULL DEFAULT '["*"]'::jsonb
        );
        INSERT INTO personal_access_tokens (name, scopes) VALUES
            ('full', '["*"]'),
            ('default', DEFAULT),
            ('read', '["read"]'),
            ('read_write', '["read", "write"]'),
            ('empty', '[]'),
            ('star_and_read', '["*", "read"]'),
            ('not_an_array', '"*"');
        "#,
    )
    .execute(&mut *transaction)
    .await
    .expect("seed tokens");

    sqlx::raw_sql(include_str!(
        "../../migrations/20261012025005_drop_narrow_personal_access_tokens.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run migration");

    let rows = sqlx::query("SELECT name FROM personal_access_tokens ORDER BY name")
        .fetch_all(&mut *transaction)
        .await
        .expect("read remaining tokens");
    let names: Vec<String> = rows.iter().map(|row| row.get("name")).collect();
    assert_eq!(names, vec!["default", "full"]);

    transaction.rollback().await.expect("roll back");
}
