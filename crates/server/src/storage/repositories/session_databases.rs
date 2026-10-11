// PostgreSQL repository: session SQL databases and their stored pages.
//
// The page cache and diffing live in `crate::session_sqldb::postgres`; this
// file only owns the SQL. A database's pages are rewritten by
// `commit_session_database_pages`, which is a compare-and-set on
// `generation` (migration 218) so two processes caching the same database
// cannot overwrite each other's pages.
//
// Decision: metadata reads and writes go through `self.pool`, joining the
// command transaction like every repository method. Page loads and page
// commits use the raw pool instead: a cache in some process is updated the
// moment a commit returns, so that commit must be a real one, never a
// savepoint inside a command transaction that could still roll back.

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_server_macros::sql;
use uuid::Uuid;

/// Pages per upsert statement, so a first write of a large database does not
/// become one multi-megabyte bind.
const PAGE_UPSERT_CHUNK: usize = 256;

/// Metadata row of a session database.
#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
pub struct SessionDatabaseRow {
    pub id: Uuid,
    pub name: String,
    pub size_bytes: i64,
    pub page_count: i32,
    pub generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Outcome of creating a session database.
pub enum CreateSessionDatabase {
    Created(SessionDatabaseRow),
    AlreadyExists,
    LimitReached,
}

impl Database {
    pub async fn get_session_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<SessionDatabaseRow>> {
        let row = sqlx::query_as::<_, SessionDatabaseRow>(sql!(
            "SELECT {SessionDatabaseRow} FROM session_databases WHERE session_id = $1 AND name = $2"
        ))
        .bind(session_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_session_databases(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<SessionDatabaseRow>> {
        let rows = sqlx::query_as::<_, SessionDatabaseRow>(sql!(
            "SELECT {SessionDatabaseRow} FROM session_databases WHERE session_id = $1 ORDER BY name"
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Create an empty database unless the name is taken or the session
    /// already holds `max_databases`.
    pub async fn create_session_database(
        &self,
        session_id: SessionId,
        name: &str,
        max_databases: i64,
    ) -> Result<CreateSessionDatabase> {
        let created = sqlx::query_as::<_, SessionDatabaseRow>(sql!(
            r#"
            INSERT INTO session_databases (session_id, name)
            SELECT $1, $2
            WHERE (SELECT COUNT(*) FROM session_databases WHERE session_id = $1) < $3
            ON CONFLICT (session_id, name) DO NOTHING
            RETURNING {SessionDatabaseRow}
            "#
        ))
        .bind(session_id)
        .bind(name)
        .bind(max_databases)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match created {
            Some(row) => CreateSessionDatabase::Created(row),
            None if self.get_session_database(session_id, name).await?.is_some() => {
                CreateSessionDatabase::AlreadyExists
            }
            None => CreateSessionDatabase::LimitReached,
        })
    }

    /// Delete a database; its pages go with it through the FK cascade.
    pub async fn delete_session_database(&self, session_id: SessionId, name: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM session_databases WHERE session_id = $1 AND name = $2")
                .bind(session_id)
                .bind(name)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Stored size of every other database of the session.
    pub async fn session_databases_size_excluding(
        &self,
        session_id: SessionId,
        database_id: Uuid,
    ) -> Result<i64> {
        let size: Option<i64> = sqlx::query_scalar(
            "SELECT SUM(size_bytes)::BIGINT FROM session_databases WHERE session_id = $1 AND id <> $2",
        )
        .bind(session_id)
        .bind(database_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(size.unwrap_or(0))
    }

    /// The database's generation and its pages in order, read in one
    /// statement so both come from the same snapshot. `None` once deleted.
    pub async fn load_session_database_pages(
        &self,
        database_id: Uuid,
    ) -> Result<Option<(i64, Vec<(i32, Vec<u8>)>)>> {
        let rows: Vec<(i64, Option<i32>, Option<Vec<u8>>)> = sqlx::query_as(
            r#"
            SELECT d.generation, p.page_number, p.data
            FROM session_databases d
            LEFT JOIN session_database_pages p ON p.database_id = d.id
            WHERE d.id = $1
            ORDER BY p.page_number
            "#,
        )
        .bind(database_id)
        .fetch_all(self.pool())
        .await?;
        let Some(generation) = rows.first().map(|row| row.0) else {
            return Ok(None);
        };
        let pages = rows
            .into_iter()
            .filter_map(|(_, number, data)| number.zip(data))
            .collect();
        Ok(Some((generation, pages)))
    }

    /// Persist one write: upsert `changed` pages, drop pages past
    /// `page_count`, and bump the generation, all in one transaction.
    ///
    /// Returns the new generation, or `None` when the stored generation is no
    /// longer `expected_generation` (another writer committed first, or the
    /// database was deleted); nothing is written then.
    pub async fn commit_session_database_pages(
        &self,
        database_id: Uuid,
        expected_generation: i64,
        changed: &[(i32, Vec<u8>)],
        page_count: i32,
        size_bytes: i64,
    ) -> Result<Option<i64>> {
        let mut tx = self.pool().begin().await?;
        // The UPDATE takes the row lock first, so a concurrent committer waits
        // here and then sees the bumped generation and matches nothing.
        let generation: Option<i64> = sqlx::query_scalar(
            r#"
            UPDATE session_databases
            SET generation = generation + 1, size_bytes = $3, page_count = $4
            WHERE id = $1 AND generation = $2
            RETURNING generation
            "#,
        )
        .bind(database_id)
        .bind(expected_generation)
        .bind(size_bytes)
        .bind(page_count)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(generation) = generation else {
            return Ok(None);
        };

        for chunk in changed.chunks(PAGE_UPSERT_CHUNK) {
            let numbers: Vec<i32> = chunk.iter().map(|(number, _)| *number).collect();
            let data: Vec<&[u8]> = chunk.iter().map(|(_, bytes)| bytes.as_slice()).collect();
            sqlx::query(
                r#"
                INSERT INTO session_database_pages (database_id, page_number, data)
                SELECT $1, page.number, page.data
                FROM UNNEST($2::INTEGER[], $3::BYTEA[]) AS page(number, data)
                ON CONFLICT (database_id, page_number) DO UPDATE SET data = EXCLUDED.data
                "#,
            )
            .bind(database_id)
            .bind(&numbers)
            .bind(&data)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "DELETE FROM session_database_pages WHERE database_id = $1 AND page_number > $2",
        )
        .bind(database_id)
        .bind(page_count)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(generation))
    }

    /// Copy every database of `source` into `target` (session fork), page for
    /// page. One statement, so the copy is a consistent snapshot even if the
    /// source is written concurrently. A name the target already has is kept.
    pub async fn copy_session_databases(
        &self,
        source_session_id: SessionId,
        target_session_id: SessionId,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            WITH source AS (
                SELECT id, name, size_bytes, page_count
                FROM session_databases
                WHERE session_id = $1
            ), copied AS (
                INSERT INTO session_databases (session_id, name, size_bytes, page_count)
                SELECT $2, name, size_bytes, page_count FROM source
                ON CONFLICT (session_id, name) DO NOTHING
                RETURNING id, name
            )
            INSERT INTO session_database_pages (database_id, page_number, data)
            SELECT copied.id, page.page_number, page.data
            FROM copied
            JOIN source ON source.name = copied.name
            JOIN session_database_pages page ON page.database_id = source.id
            "#,
        )
        .bind(source_session_id)
        .bind(target_session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
