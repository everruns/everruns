// PostgreSQL repository: room files (knowledge/runtime-resources/rooms.md).
//
// Decisions:
// - A room's current files are rows of its Memory (`memory_files`), so an
//   agent's room session mounts them at `/room` with no copy. Every change also
//   appends a `room_file_versions` row and a room event, in the same
//   transaction as the change, so history and the change feed never disagree
//   with the files.
// - Changes to one room's files are serialized by a transaction-scoped
//   advisory lock. Room files are small and edited by a handful of members, so
//   one writer at a time per room costs nothing and makes version numbers and
//   the "expected version" check exact.
// - A file keeps its identity (`file_id`, the `memory_files` row id) across
//   moves, and a restore of a deleted file reuses it, so one history follows
//   the file wherever it goes.

use super::rows::*;
use super::{Database, insert_event};
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

/// Every ancestor directory of a normalized path, shallowest first:
/// `/a/b/c.md` gives `/a`, `/a/b`.
fn ancestors(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(next) = path[at + 1..].find('/') {
        at += next + 1;
        out.push(path[..at].to_string());
    }
    out
}

struct Current {
    id: Uuid,
    is_directory: bool,
    version: Option<i32>,
}

impl Database {
    /// The Memory that holds a room's files, created on first use.
    pub async fn ensure_room_memory(
        &self,
        org_id: i64,
        room_id: RoomIdRef,
    ) -> Result<RoomFileStore> {
        sqlx::query(
            r#"
            INSERT INTO memories (org_id, public_id, name, description, scope, owner_room_id)
            VALUES ($1, $2, $3, $4, 'room', $5)
            ON CONFLICT (org_id, owner_room_id) WHERE scope = 'room' AND status <> 'deleted'
            DO NOTHING
            "#,
        )
        .bind(org_id)
        .bind(crate::kernel_imports::contracts::typed_id::MemoryId::new().to_string())
        .bind(format!("room-{}", room_id.uuid().simple()))
        .bind("Shared files of one room.")
        .bind(room_id)
        .execute(&self.pool)
        .await?;
        let memory_id: Uuid = sqlx::query_scalar(
            r#"
            SELECT id FROM memories
            WHERE org_id = $1 AND scope = 'room' AND owner_room_id = $2 AND status <> 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(room_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RoomFileStore {
            room_id,
            org_id,
            memory_id,
        })
    }

    /// The room a room-scoped Memory belongs to.
    pub async fn room_of_memory(&self, memory_id: Uuid) -> Result<Option<RoomIdRef>> {
        Ok(sqlx::query_scalar(
            "SELECT owner_room_id FROM memories WHERE id = $1 AND scope = 'room'",
        )
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// The room's files as they are now, each with its latest version.
    pub async fn list_room_files(&self, store: RoomFileStore) -> Result<Vec<RoomFileRow>> {
        Ok(sqlx::query_as::<_, RoomFileRow>(
            r#"
            SELECT f.id AS file_id, f.path, f.size_bytes, f.updated_at,
                   COALESCE(v.version, 0) AS version, v.author_member_id
            FROM memory_files f
            LEFT JOIN LATERAL (
                SELECT version, author_member_id FROM room_file_versions
                WHERE file_id = f.id ORDER BY version DESC LIMIT 1
            ) v ON TRUE
            WHERE f.memory_id = $1 AND NOT f.is_directory
            ORDER BY f.path
            "#,
        )
        .bind(store.memory_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The file that is, or last was, at `path`: its id, whether it exists
    /// now, and its latest version.
    pub async fn find_room_file(
        &self,
        store: RoomFileStore,
        path: &str,
    ) -> Result<Option<(Uuid, Option<RoomFileVersionRow>)>> {
        let live: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM memory_files WHERE memory_id = $1 AND path = $2 AND NOT is_directory",
        )
        .bind(store.memory_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;
        let file_id = match live {
            Some(id) => Some(id),
            None => {
                sqlx::query_scalar(
                    r#"
                SELECT file_id FROM room_file_versions
                WHERE room_id = $1 AND path = $2
                ORDER BY created_at DESC, version DESC LIMIT 1
                "#,
                )
                .bind(store.room_id)
                .bind(path)
                .fetch_optional(&self.pool)
                .await?
            }
        };
        let Some(file_id) = file_id else {
            return Ok(None);
        };
        let latest = sqlx::query_as::<_, RoomFileVersionRow>(sql!(
            r#"
            SELECT {RoomFileVersionRow} FROM room_file_versions
            WHERE file_id = $1 ORDER BY version DESC LIMIT 1
            "#
        ))
        .bind(file_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(Some((file_id, latest)))
    }

    /// Current content of a file, if there is one at `path`.
    pub async fn read_room_file(
        &self,
        store: RoomFileStore,
        path: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar::<_, Option<Vec<u8>>>(
            "SELECT content FROM memory_files WHERE memory_id = $1 AND path = $2 AND NOT is_directory",
        )
        .bind(store.memory_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?
        .map(Option::unwrap_or_default))
    }

    /// Every version of one file, newest first.
    pub async fn list_room_file_versions(
        &self,
        room_id: RoomIdRef,
        file_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoomFileVersionRow>> {
        Ok(sqlx::query_as::<_, RoomFileVersionRow>(sql!(
            r#"
            SELECT {RoomFileVersionRow} FROM room_file_versions
            WHERE room_id = $1 AND file_id = $2
            ORDER BY version DESC
            LIMIT $3
            "#
        ))
        .bind(room_id)
        .bind(file_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// One version of a file, with the content it had.
    pub async fn get_room_file_version(
        &self,
        room_id: RoomIdRef,
        file_id: Uuid,
        version: i32,
    ) -> Result<Option<(RoomFileVersionRow, Option<Vec<u8>>)>> {
        let row = sqlx::query_as::<_, RoomFileVersionRow>(sql!(
            r#"
            SELECT {RoomFileVersionRow} FROM room_file_versions
            WHERE room_id = $1 AND file_id = $2 AND version = $3
            "#
        ))
        .bind(room_id)
        .bind(file_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let content: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM room_file_versions WHERE id = $1")
                .bind(row.id)
                .fetch_one(&self.pool)
                .await?;
        Ok(Some((row, content)))
    }

    /// Write a file's whole content, creating it (and its folders) when it
    /// is not there. `expected` is the version the writer read: `Some(0)`
    /// means "only if there is no file", `None` means "whatever is there".
    /// `action` is `write` or `restore`; a restore of a deleted file passes
    /// its old `file_id` so the history continues.
    #[allow(clippy::too_many_arguments)]
    pub async fn write_room_file(
        &self,
        store: RoomFileStore,
        path: &str,
        content: &[u8],
        expected: Option<i32>,
        action: &str,
        revive: Option<Uuid>,
        author: RoomFileAuthor,
    ) -> Result<RoomFileChange> {
        let mut tx = self.pool.begin().await?;
        lock_room_files(&mut tx, store).await?;
        let current = current_file(&mut tx, store, path).await?;
        if current.as_ref().is_some_and(|c| c.is_directory) {
            return Ok(RoomFileChange::WrongKind(format!("{path} is a folder")));
        }
        let at = current.as_ref().map(|c| c.version.unwrap_or(0));
        let stale = match expected {
            None => false,
            Some(0) => current.is_some(),
            Some(expected) => at != Some(expected),
        };
        if stale {
            return Ok(RoomFileChange::Conflict { current: at });
        }
        if let Some(blocked) = ensure_folders(&mut tx, store, path).await? {
            return Ok(RoomFileChange::WrongKind(format!("{blocked} is a file")));
        }
        let file_id = match &current {
            Some(file) => {
                sqlx::query(
                    r#"
                    UPDATE memory_files
                    SET content = $2, size_bytes = $3, content_hash = NULL, updated_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(file.id)
                .bind(content)
                .bind(content.len() as i64)
                .execute(&mut *tx)
                .await?;
                file.id
            }
            None => {
                sqlx::query_scalar(
                    r#"
                    INSERT INTO memory_files (id, memory_id, path, content, is_directory, size_bytes)
                    VALUES (COALESCE($1, public.uuidv7()), $2, $3, $4, FALSE, $5)
                    RETURNING id
                    "#,
                )
                .bind(revive)
                .bind(store.memory_id)
                .bind(path)
                .bind(content)
                .bind(content.len() as i64)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        let version = insert_version(
            &mut tx,
            store,
            file_id,
            path,
            action,
            Some(content),
            None,
            author,
        )
        .await?;
        finish(tx, store, &version, "file.written").await
    }

    /// Delete a file. Its versions stay; reading an old one or restoring it
    /// still works.
    pub async fn delete_room_file(
        &self,
        store: RoomFileStore,
        path: &str,
        expected: Option<i32>,
        author: RoomFileAuthor,
    ) -> Result<RoomFileChange> {
        let mut tx = self.pool.begin().await?;
        lock_room_files(&mut tx, store).await?;
        let Some(file) = current_file(&mut tx, store, path).await? else {
            return Ok(RoomFileChange::NotFound);
        };
        if file.is_directory {
            return Ok(RoomFileChange::WrongKind(format!("{path} is a folder")));
        }
        if let Some(expected) = expected
            && Some(expected) != file.version
        {
            return Ok(RoomFileChange::Conflict {
                current: file.version,
            });
        }
        sqlx::query("DELETE FROM memory_files WHERE id = $1")
            .bind(file.id)
            .execute(&mut *tx)
            .await?;
        let version =
            insert_version(&mut tx, store, file.id, path, "delete", None, None, author).await?;
        finish(tx, store, &version, "file.deleted").await
    }

    /// Move (rename) a file. The file keeps its history.
    pub async fn move_room_file(
        &self,
        store: RoomFileStore,
        from: &str,
        to: &str,
        author: RoomFileAuthor,
    ) -> Result<RoomFileChange> {
        let mut tx = self.pool.begin().await?;
        lock_room_files(&mut tx, store).await?;
        let Some(file) = current_file(&mut tx, store, from).await? else {
            return Ok(RoomFileChange::NotFound);
        };
        if file.is_directory {
            return Ok(RoomFileChange::WrongKind(format!("{from} is a folder")));
        }
        if current_file(&mut tx, store, to).await?.is_some() {
            return Ok(RoomFileChange::WrongKind(format!("{to} already exists")));
        }
        if let Some(blocked) = ensure_folders(&mut tx, store, to).await? {
            return Ok(RoomFileChange::WrongKind(format!("{blocked} is a file")));
        }
        let content: Option<Vec<u8>> = sqlx::query_scalar(
            "UPDATE memory_files SET path = $2, updated_at = NOW() WHERE id = $1 RETURNING content",
        )
        .bind(file.id)
        .bind(to)
        .fetch_one(&mut *tx)
        .await?;
        let version = insert_version(
            &mut tx,
            store,
            file.id,
            to,
            "move",
            Some(content.as_deref().unwrap_or_default()),
            Some(from),
            author,
        )
        .await?;
        finish(tx, store, &version, "file.moved").await
    }
}

/// Alias so signatures read as the room id they take.
type RoomIdRef = crate::kernel_imports::contracts::typed_id::RoomId;

async fn lock_room_files(conn: &mut sqlx::PgConnection, store: RoomFileStore) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('room_files:' || $1::text, 0))")
        .bind(store.room_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn current_file(
    conn: &mut sqlx::PgConnection,
    store: RoomFileStore,
    path: &str,
) -> Result<Option<Current>> {
    let row: Option<(Uuid, bool, Option<i32>)> = sqlx::query_as(
        r#"
        SELECT f.id, f.is_directory,
               (SELECT MAX(version) FROM room_file_versions WHERE file_id = f.id)
        FROM memory_files f
        WHERE f.memory_id = $1 AND f.path = $2
        "#,
    )
    .bind(store.memory_id)
    .bind(path)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(id, is_directory, version)| Current {
        id,
        is_directory,
        version,
    }))
}

/// Create the folders a path needs. Returns the first ancestor that is a
/// file, when there is one, and creates nothing then.
async fn ensure_folders(
    conn: &mut sqlx::PgConnection,
    store: RoomFileStore,
    path: &str,
) -> Result<Option<String>> {
    let folders = ancestors(path);
    if folders.is_empty() {
        return Ok(None);
    }
    let blocked: Option<String> = sqlx::query_scalar(
        r#"
        SELECT path FROM memory_files
        WHERE memory_id = $1 AND path = ANY($2) AND NOT is_directory
        ORDER BY length(path) LIMIT 1
        "#,
    )
    .bind(store.memory_id)
    .bind(&folders)
    .fetch_optional(&mut *conn)
    .await?;
    if blocked.is_some() {
        return Ok(blocked);
    }
    sqlx::query(
        r#"
        INSERT INTO memory_files (memory_id, path, is_directory, size_bytes)
        SELECT $1, folder, TRUE, 0 FROM unnest($2::text[]) AS folder
        ON CONFLICT (memory_id, path) DO NOTHING
        "#,
    )
    .bind(store.memory_id)
    .bind(&folders)
    .execute(&mut *conn)
    .await?;
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
async fn insert_version(
    conn: &mut sqlx::PgConnection,
    store: RoomFileStore,
    file_id: Uuid,
    path: &str,
    action: &str,
    content: Option<&[u8]>,
    moved_from: Option<&str>,
    author: RoomFileAuthor,
) -> Result<RoomFileVersionRow> {
    Ok(sqlx::query_as::<_, RoomFileVersionRow>(sql!(
        r#"
        INSERT INTO room_file_versions
            (room_id, org_id, file_id, path, version, action, content, size_bytes,
             moved_from, author_member_id, session_id)
        VALUES ($1, $2, $3, $4,
                (SELECT COALESCE(MAX(version), 0) + 1 FROM room_file_versions WHERE file_id = $3),
                $5, $6, $7, $8, $9, $10)
        RETURNING {RoomFileVersionRow}
        "#
    ))
    .bind(store.room_id)
    .bind(store.org_id)
    .bind(file_id)
    .bind(path)
    .bind(action)
    .bind(content)
    .bind(content.map_or(0, |c| c.len() as i64))
    .bind(moved_from)
    .bind(author.member_id)
    .bind(author.session_id)
    .fetch_one(&mut *conn)
    .await?)
}

async fn finish(
    mut tx: crate::storage::transaction::DbTx,
    store: RoomFileStore,
    version: &RoomFileVersionRow,
    event_type: &str,
) -> Result<RoomFileChange> {
    insert_event(
        &mut tx,
        CreateRoomEventRow {
            room_id: store.room_id,
            org_id: store.org_id,
            event_type: event_type.into(),
            message_id: None,
            member_id: version.author_member_id,
            data: serde_json::json!({
                "path": version.path,
                "version": version.version,
                "file_id": version.file_id,
                "moved_from": version.moved_from,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(RoomFileChange::Done(version.clone()))
}

#[cfg(test)]
mod tests {
    use super::ancestors;

    #[test]
    fn ancestors_are_every_folder_on_the_way_down() {
        assert_eq!(ancestors("/a/b/c.md"), vec!["/a", "/a/b"]);
        assert!(ancestors("/c.md").is_empty());
    }
}
