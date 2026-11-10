// PostgreSQL repository: Rooms (knowledge/runtime-resources/rooms.md).
//
// Decisions:
// - Posting a message, bumping its thread's reply count, moving the room's
//   last activity and appending the room event happen in one transaction, so
//   the change feed never names a message that is not there (or misses one).
// - Nothing is purged. Archive, leave and delete set timestamps.
// - Read marks only move forward (`GREATEST`), so a slow client cannot mark
//   messages unread again by sending an older position.

pub(super) mod rows;
use rows::*;

use super::Database;
use crate::kernel_imports::contracts::typed_id::{AgentId, RoomId, RoomMemberId, RoomMessageId};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_server_macros::sql;
use uuid::Uuid;

/// `RoomMessageRow`'s columns qualified with a table alias, for joins.
fn message_columns(alias: &str) -> String {
    RoomMessageRow::COLUMNS
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Database {
    // ============================================
    // Rooms
    // ============================================

    pub async fn create_room(&self, input: CreateRoomRow) -> Result<RoomRow> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomRow>(sql!(
            r#"
            INSERT INTO rooms (org_id, name, topic, external_ref, metadata, link_template, settings,
                               created_by_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING {RoomRow}
            "#
        ))
        .bind(input.org_id)
        .bind(&input.name)
        .bind(&input.topic)
        .bind(&input.external_ref)
        .bind(&input.metadata)
        .bind(&input.link_template)
        .bind(&input.settings)
        .bind(input.created_by_user_id)
        .fetch_one(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            CreateRoomEventRow {
                room_id: row.id,
                org_id: row.org_id,
                event_type: "room.created".into(),
                message_id: None,
                member_id: None,
                data: serde_json::json!({ "name": row.name }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn get_room(&self, org_id: i64, room_id: RoomId) -> Result<Option<RoomRow>> {
        Ok(sqlx::query_as::<_, RoomRow>(sql!(
            "SELECT {RoomRow} FROM rooms WHERE org_id = $1 AND id = $2"
        ))
        .bind(org_id)
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_room_by_external_ref(
        &self,
        org_id: i64,
        external_ref: &str,
    ) -> Result<Option<RoomRow>> {
        Ok(sqlx::query_as::<_, RoomRow>(sql!(
            "SELECT {RoomRow} FROM rooms WHERE org_id = $1 AND external_ref = $2"
        ))
        .bind(org_id)
        .bind(external_ref)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Rooms of an org, most recently active first. `member_user_id` narrows
    /// to rooms that user is a member of.
    pub async fn list_rooms(
        &self,
        org_id: i64,
        include_archived: bool,
        member_user_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<RoomRow>> {
        Ok(sqlx::query_as::<_, RoomRow>(sql!(
            r#"
            SELECT {RoomRow} FROM rooms r
            WHERE r.org_id = $1
              AND ($2 OR r.archived_at IS NULL)
              AND ($3::uuid IS NULL OR EXISTS (
                    SELECT 1 FROM room_members m
                    WHERE m.room_id = r.id AND m.user_id = $3 AND m.left_at IS NULL))
            ORDER BY COALESCE(r.last_message_at, r.created_at) DESC
            LIMIT $4
            "#
        ))
        .bind(org_id)
        .bind(include_archived)
        .bind(member_user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn update_room(
        &self,
        org_id: i64,
        room_id: RoomId,
        input: UpdateRoomRow,
    ) -> Result<Option<RoomRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomRow>(sql!(
            r#"
            UPDATE rooms SET
                name = COALESCE($3, name),
                topic = CASE WHEN $4 THEN $5 ELSE topic END,
                metadata = COALESCE($6, metadata),
                link_template = CASE WHEN $7 THEN $8 ELSE link_template END,
                settings = COALESCE($9, settings),
                updated_at = now()
            WHERE org_id = $1 AND id = $2
            RETURNING {RoomRow}
            "#
        ))
        .bind(org_id)
        .bind(room_id)
        .bind(&input.name)
        .bind(input.topic.is_some())
        .bind(input.topic.clone().flatten())
        .bind(&input.metadata)
        .bind(input.link_template.is_some())
        .bind(input.link_template.clone().flatten())
        .bind(&input.settings)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            insert_event(
                &mut tx,
                CreateRoomEventRow {
                    room_id: row.id,
                    org_id: row.org_id,
                    event_type: "room.updated".into(),
                    message_id: None,
                    member_id: None,
                    data: serde_json::json!({ "name": row.name, "topic": row.topic }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    pub async fn set_room_archived(
        &self,
        org_id: i64,
        room_id: RoomId,
        archived: bool,
    ) -> Result<Option<RoomRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomRow>(sql!(
            r#"
            UPDATE rooms SET
                archived_at = CASE WHEN $3 THEN COALESCE(archived_at, now()) ELSE NULL END,
                updated_at = now()
            WHERE org_id = $1 AND id = $2
            RETURNING {RoomRow}
            "#
        ))
        .bind(org_id)
        .bind(room_id)
        .bind(archived)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            insert_event(
                &mut tx,
                CreateRoomEventRow {
                    room_id: row.id,
                    org_id: row.org_id,
                    event_type: if archived {
                        "room.archived"
                    } else {
                        "room.unarchived"
                    }
                    .into(),
                    message_id: None,
                    member_id: None,
                    data: serde_json::json!({}),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    // ============================================
    // Members
    // ============================================

    pub async fn add_room_member(&self, input: CreateRoomMemberRow) -> Result<RoomMemberRow> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            INSERT INTO room_members (room_id, org_id, kind, user_id, agent_id, handle, display_name,
                                      role, wake, session_scope, instructions)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING {RoomMemberRow}
            "#
        ))
        .bind(input.room_id)
        .bind(input.org_id)
        .bind(&input.kind)
        .bind(input.user_id)
        .bind(input.agent_id)
        .bind(&input.handle)
        .bind(&input.display_name)
        .bind(&input.role)
        .bind(&input.wake)
        .bind(&input.session_scope)
        .bind(&input.instructions)
        .fetch_one(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            CreateRoomEventRow {
                room_id: row.room_id,
                org_id: row.org_id,
                event_type: "member.joined".into(),
                message_id: None,
                member_id: Some(row.id),
                data: serde_json::json!({ "handle": row.handle, "kind": row.kind }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn list_room_members(
        &self,
        room_id: RoomId,
        include_left: bool,
    ) -> Result<Vec<RoomMemberRow>> {
        Ok(sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            SELECT {RoomMemberRow} FROM room_members
            WHERE room_id = $1 AND ($2 OR left_at IS NULL)
            ORDER BY created_at
            "#
        ))
        .bind(room_id)
        .bind(include_left)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A member by id, including one who left (their old messages still
    /// name them).
    pub async fn get_room_member(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
    ) -> Result<Option<RoomMemberRow>> {
        Ok(sqlx::query_as::<_, RoomMemberRow>(sql!(
            "SELECT {RoomMemberRow} FROM room_members WHERE room_id = $1 AND id = $2"
        ))
        .bind(room_id)
        .bind(member_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn find_room_member_by_user(
        &self,
        room_id: RoomId,
        user_id: Uuid,
    ) -> Result<Option<RoomMemberRow>> {
        Ok(sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            SELECT {RoomMemberRow} FROM room_members
            WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL
            "#
        ))
        .bind(room_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn find_room_member_by_agent(
        &self,
        room_id: RoomId,
        agent_id: AgentId,
    ) -> Result<Option<RoomMemberRow>> {
        Ok(sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            SELECT {RoomMemberRow} FROM room_members
            WHERE room_id = $1 AND agent_id = $2 AND left_at IS NULL
            "#
        ))
        .bind(room_id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_room_member(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
        input: UpdateRoomMemberRow,
    ) -> Result<Option<RoomMemberRow>> {
        Ok(sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            UPDATE room_members SET
                display_name = COALESCE($3, display_name),
                role = COALESCE($4, role),
                wake = COALESCE($5, wake),
                session_scope = COALESCE($6, session_scope),
                instructions = CASE WHEN $7 THEN $8 ELSE instructions END,
                updated_at = now()
            WHERE room_id = $1 AND id = $2 AND left_at IS NULL
            RETURNING {RoomMemberRow}
            "#
        ))
        .bind(room_id)
        .bind(member_id)
        .bind(&input.display_name)
        .bind(&input.role)
        .bind(&input.wake)
        .bind(&input.session_scope)
        .bind(input.instructions.is_some())
        .bind(input.instructions.clone().flatten())
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn remove_room_member(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
    ) -> Result<Option<RoomMemberRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomMemberRow>(sql!(
            r#"
            UPDATE room_members SET left_at = now(), updated_at = now()
            WHERE room_id = $1 AND id = $2 AND left_at IS NULL
            RETURNING {RoomMemberRow}
            "#
        ))
        .bind(room_id)
        .bind(member_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            insert_event(
                &mut tx,
                CreateRoomEventRow {
                    room_id: row.room_id,
                    org_id: row.org_id,
                    event_type: "member.left".into(),
                    message_id: None,
                    member_id: Some(row.id),
                    data: serde_json::json!({ "handle": row.handle }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    // ============================================
    // Messages
    // ============================================

    /// Post a message: insert it, count it on its thread, move the room's
    /// activity and append `message.posted`, all at once.
    ///
    /// A message with a `source_ref` the room already has is not posted again:
    /// `None` comes back instead.
    pub async fn post_room_message(
        &self,
        input: CreateRoomMessageRow,
    ) -> Result<Option<RoomMessageRow>> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            INSERT INTO room_messages (room_id, org_id, author_member_id, thread_id, text, parts,
                                       mentions, hops, session_id, source_ref)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (room_id, source_ref) WHERE source_ref IS NOT NULL DO NOTHING
            RETURNING {RoomMessageRow}
            "#
        ))
        .bind(input.room_id)
        .bind(input.org_id)
        .bind(input.author_member_id)
        .bind(input.thread_id)
        .bind(&input.text)
        .bind(&input.parts)
        .bind(&input.mentions)
        .bind(input.hops)
        .bind(input.session_id)
        .bind(&input.source_ref)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        if let Some(thread_id) = row.thread_id {
            sqlx::query(
                r#"
                UPDATE room_messages
                SET reply_count = reply_count + 1, last_reply_at = $2
                WHERE id = $1
                "#,
            )
            .bind(thread_id)
            .bind(row.created_at)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE rooms SET last_message_at = $2 WHERE id = $1")
            .bind(row.room_id)
            .bind(row.created_at)
            .execute(&mut *tx)
            .await?;
        insert_event(
            &mut tx,
            CreateRoomEventRow {
                room_id: row.room_id,
                org_id: row.org_id,
                event_type: "message.posted".into(),
                message_id: Some(row.id),
                member_id: row.author_member_id,
                data: serde_json::json!({
                    "thread_id": row.thread_id,
                    "mentions": row.mentions,
                }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn get_room_message(
        &self,
        room_id: RoomId,
        message_id: RoomMessageId,
    ) -> Result<Option<RoomMessageRow>> {
        Ok(sqlx::query_as::<_, RoomMessageRow>(sql!(
            "SELECT {RoomMessageRow} FROM room_messages WHERE room_id = $1 AND id = $2"
        ))
        .bind(room_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// One page of a room's top level (`thread_id` None) or of one thread,
    /// oldest first. `before` pages backwards from the newest; `after` reads
    /// forward from a position. Deleted messages stay in place as markers.
    pub async fn list_room_messages(
        &self,
        room_id: RoomId,
        thread_id: Option<RoomMessageId>,
        before: Option<DateTime<Utc>>,
        after: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<RoomMessageRow>> {
        // Forward reads take the oldest `limit` after the position; everything
        // else the newest `limit`, flipped back to reading order.
        let forward = after.is_some() && before.is_none();
        let mut rows = sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            SELECT {RoomMessageRow} FROM room_messages
            WHERE room_id = $1
              AND (($2::uuid IS NULL AND thread_id IS NULL) OR thread_id = $2)
              AND ($3::timestamptz IS NULL OR created_at < $3)
              AND ($4::timestamptz IS NULL OR created_at > $4)
            ORDER BY
              CASE WHEN $6 THEN created_at END ASC,
              CASE WHEN NOT $6 THEN created_at END DESC
            LIMIT $5
            "#
        ))
        .bind(room_id)
        .bind(thread_id)
        .bind(before)
        .bind(after)
        .bind(limit)
        .bind(forward)
        .fetch_all(&self.pool)
        .await?;
        if !forward {
            rows.reverse();
        }
        Ok(rows)
    }

    pub async fn list_room_messages_by_ids(
        &self,
        room_id: RoomId,
        ids: &[Uuid],
    ) -> Result<Vec<RoomMessageRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, RoomMessageRow>(sql!(
            "SELECT {RoomMessageRow} FROM room_messages WHERE room_id = $1 AND id = ANY($2)"
        ))
        .bind(room_id)
        .bind(ids)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn list_pinned_room_messages(&self, room_id: RoomId) -> Result<Vec<RoomMessageRow>> {
        Ok(sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            SELECT {RoomMessageRow} FROM room_messages
            WHERE room_id = $1 AND pinned_at IS NOT NULL AND deleted_at IS NULL
            ORDER BY pinned_at DESC
            LIMIT 100
            "#
        ))
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Case-insensitive text search over a room's live messages, newest first.
    pub async fn search_room_messages(
        &self,
        room_id: RoomId,
        query: &str,
        limit: i64,
    ) -> Result<Vec<RoomMessageRow>> {
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        Ok(sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            SELECT {RoomMessageRow} FROM room_messages
            WHERE room_id = $1 AND deleted_at IS NULL AND text ILIKE $2
            ORDER BY created_at DESC
            LIMIT $3
            "#
        ))
        .bind(room_id)
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Change a live message's text and mentions; appends `message.edited`.
    pub async fn edit_room_message(
        &self,
        room_id: RoomId,
        message_id: RoomMessageId,
        text: &str,
        parts: &serde_json::Value,
        mentions: &[Uuid],
    ) -> Result<Option<RoomMessageRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            UPDATE room_messages SET text = $3, parts = $4, mentions = $5, edited_at = now()
            WHERE room_id = $1 AND id = $2 AND deleted_at IS NULL
            RETURNING {RoomMessageRow}
            "#
        ))
        .bind(room_id)
        .bind(message_id)
        .bind(text)
        .bind(parts)
        .bind(mentions)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            insert_event(&mut tx, message_event(row, "message.edited")).await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    /// Mark a message deleted. The row stays (and keeps its text, for the
    /// record); readers show a marker in its place.
    pub async fn delete_room_message(
        &self,
        room_id: RoomId,
        message_id: RoomMessageId,
    ) -> Result<Option<RoomMessageRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            UPDATE room_messages SET deleted_at = now(), pinned_at = NULL
            WHERE room_id = $1 AND id = $2 AND deleted_at IS NULL
            RETURNING {RoomMessageRow}
            "#
        ))
        .bind(room_id)
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            insert_event(&mut tx, message_event(row, "message.deleted")).await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    pub async fn set_room_message_pinned(
        &self,
        room_id: RoomId,
        message_id: RoomMessageId,
        pinned: bool,
        by: Option<RoomMemberId>,
    ) -> Result<Option<RoomMessageRow>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, RoomMessageRow>(sql!(
            r#"
            UPDATE room_messages
            SET pinned_at = CASE WHEN $3 THEN COALESCE(pinned_at, now()) ELSE NULL END
            WHERE room_id = $1 AND id = $2 AND deleted_at IS NULL
            RETURNING {RoomMessageRow}
            "#
        ))
        .bind(room_id)
        .bind(message_id)
        .bind(pinned)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            let mut event = message_event(
                row,
                if pinned {
                    "message.pinned"
                } else {
                    "message.unpinned"
                },
            );
            event.member_id = by;
            insert_event(&mut tx, event).await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    /// Agent posts in a room since a time: all of them, or one member's.
    pub async fn count_room_agent_posts_since(
        &self,
        room_id: RoomId,
        member_id: Option<RoomMemberId>,
        since: DateTime<Utc>,
    ) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM room_messages
            WHERE room_id = $1 AND session_id IS NOT NULL AND created_at >= $3
              AND ($2::uuid IS NULL OR author_member_id = $2)
            "#,
        )
        .bind(room_id)
        .bind(member_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await?)
    }

    // ============================================
    // Reactions
    // ============================================

    /// Add a reaction; `false` when the member already reacted so.
    pub async fn add_room_reaction(
        &self,
        room: &RoomRow,
        message_id: RoomMessageId,
        member_id: RoomMemberId,
        emoji: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let added = sqlx::query(
            r#"
            INSERT INTO room_reactions (message_id, member_id, emoji) VALUES ($1, $2, $3)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(message_id)
        .bind(member_id)
        .bind(emoji)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if added {
            insert_event(
                &mut tx,
                CreateRoomEventRow {
                    room_id: room.id,
                    org_id: room.org_id,
                    event_type: "reaction.added".into(),
                    message_id: Some(message_id),
                    member_id: Some(member_id),
                    data: serde_json::json!({ "emoji": emoji }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(added)
    }

    pub async fn remove_room_reaction(
        &self,
        room: &RoomRow,
        message_id: RoomMessageId,
        member_id: RoomMemberId,
        emoji: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let removed = sqlx::query(
            "DELETE FROM room_reactions WHERE message_id = $1 AND member_id = $2 AND emoji = $3",
        )
        .bind(message_id)
        .bind(member_id)
        .bind(emoji)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if removed {
            insert_event(
                &mut tx,
                CreateRoomEventRow {
                    room_id: room.id,
                    org_id: room.org_id,
                    event_type: "reaction.removed".into(),
                    message_id: Some(message_id),
                    member_id: Some(member_id),
                    data: serde_json::json!({ "emoji": emoji }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    pub async fn list_room_reactions(&self, message_ids: &[Uuid]) -> Result<Vec<RoomReactionRow>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, RoomReactionRow>(sql!(
            r#"
            SELECT {RoomReactionRow} FROM room_reactions
            WHERE message_id = ANY($1)
            ORDER BY created_at
            "#
        ))
        .bind(message_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    // ============================================
    // Reads
    // ============================================

    /// Move a reader's mark in one scope forward to `read_through`.
    pub async fn mark_room_read(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
        scope_id: Uuid,
        read_through: DateTime<Utc>,
    ) -> Result<RoomReadMarkRow> {
        Ok(sqlx::query_as::<_, RoomReadMarkRow>(sql!(
            r#"
            INSERT INTO room_read_marks (member_id, scope_id, room_id, read_through)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (member_id, scope_id) DO UPDATE SET
                read_through = GREATEST(room_read_marks.read_through, EXCLUDED.read_through),
                updated_at = now()
            RETURNING {RoomReadMarkRow}
            "#
        ))
        .bind(member_id)
        .bind(scope_id)
        .bind(room_id)
        .bind(read_through)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_room_read_marks(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
    ) -> Result<Vec<RoomReadMarkRow>> {
        Ok(sqlx::query_as::<_, RoomReadMarkRow>(sql!(
            "SELECT {RoomReadMarkRow} FROM room_read_marks WHERE room_id = $1 AND member_id = $2"
        ))
        .bind(room_id)
        .bind(member_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Marks in one scope at or past `at`: who has read up to there.
    pub async fn list_room_readers(
        &self,
        scope_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<Vec<RoomReadMarkRow>> {
        Ok(sqlx::query_as::<_, RoomReadMarkRow>(sql!(
            r#"
            SELECT {RoomReadMarkRow} FROM room_read_marks
            WHERE scope_id = $1 AND read_through >= $2
            ORDER BY updated_at
            "#
        ))
        .bind(scope_id)
        .bind(at)
        .fetch_all(&self.pool)
        .await?)
    }

    /// What a member has not read yet, oldest first: top-level messages past
    /// their room mark, and replies past their thread mark in threads they are
    /// involved in (started, replied to, or were mentioned in). Their own
    /// messages and deleted ones never count. With no mark yet, everything
    /// since they joined counts.
    pub async fn list_room_unread(
        &self,
        room_id: RoomId,
        member: &RoomMemberRow,
        limit: i64,
    ) -> Result<Vec<RoomMessageRow>> {
        let query = format!(
            r#"
            SELECT {columns} FROM room_messages m
            LEFT JOIN room_read_marks top
                   ON top.member_id = $2 AND top.scope_id = m.room_id
            LEFT JOIN room_read_marks th
                   ON th.member_id = $2 AND th.scope_id = m.thread_id
            WHERE m.room_id = $1
              AND m.deleted_at IS NULL
              AND m.author_member_id IS DISTINCT FROM $2
              AND (
                (m.thread_id IS NULL AND m.created_at > COALESCE(top.read_through, $3))
                OR (m.thread_id IS NOT NULL
                    AND m.created_at > COALESCE(th.read_through, $3)
                    AND EXISTS (
                        SELECT 1 FROM room_messages x
                        WHERE (x.id = m.thread_id OR x.thread_id = m.thread_id)
                          AND (x.author_member_id = $2 OR $2 = ANY(x.mentions))))
              )
            ORDER BY m.created_at
            LIMIT $4
            "#,
            columns = message_columns("m"),
        );
        // Only the compile-time column list is formatted in; every value is
        // bound.
        Ok(
            sqlx::query_as::<_, RoomMessageRow>(sqlx::AssertSqlSafe(query))
                .bind(room_id)
                .bind(member.id)
                .bind(member.created_at)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    // ============================================
    // Events
    // ============================================

    /// The change feed after `after_seq`, oldest first: one room's, or every
    /// room of the org.
    pub async fn list_room_events(
        &self,
        org_id: i64,
        room_id: Option<RoomId>,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<RoomEventRow>> {
        Ok(sqlx::query_as::<_, RoomEventRow>(sql!(
            r#"
            SELECT {RoomEventRow} FROM room_events
            WHERE org_id = $1 AND ($2::uuid IS NULL OR room_id = $2) AND seq > $3
            ORDER BY seq
            LIMIT $4
            "#
        ))
        .bind(org_id)
        .bind(room_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn append_room_event(&self, input: CreateRoomEventRow) -> Result<RoomEventRow> {
        let mut tx = self.pool.begin().await?;
        let row = insert_event(&mut tx, input).await?;
        tx.commit().await?;
        Ok(row)
    }

    // ============================================
    // Agent sessions
    // ============================================

    pub async fn get_room_agent_session(
        &self,
        member_id: RoomMemberId,
        scope_id: Uuid,
    ) -> Result<Option<RoomAgentSessionRow>> {
        Ok(sqlx::query_as::<_, RoomAgentSessionRow>(sql!(
            r#"
            SELECT {RoomAgentSessionRow} FROM room_agent_sessions
            WHERE member_id = $1 AND scope_id = $2
            "#
        ))
        .bind(member_id)
        .bind(scope_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Bind a session to a member's scope. When another wake bound one first,
    /// that binding wins and is returned.
    pub async fn bind_room_agent_session(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
        scope_id: Uuid,
        session_id: crate::kernel_imports::contracts::typed_id::SessionId,
    ) -> Result<RoomAgentSessionRow> {
        sqlx::query(
            r#"
            INSERT INTO room_agent_sessions (member_id, scope_id, room_id, session_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (member_id, scope_id) DO NOTHING
            "#,
        )
        .bind(member_id)
        .bind(scope_id)
        .bind(room_id)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        self.get_room_agent_session(member_id, scope_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("room agent session binding vanished"))
    }

    /// Remember the message that just woke the agent in this scope.
    pub async fn record_room_agent_wake(
        &self,
        member_id: RoomMemberId,
        scope_id: Uuid,
        message: &RoomMessageRow,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE room_agent_sessions
            SET last_wake_message_id = $3, last_wake_thread_id = $4, last_wake_hops = $5
            WHERE member_id = $1 AND scope_id = $2
            "#,
        )
        .bind(member_id)
        .bind(scope_id)
        .bind(message.id)
        .bind(message.thread_id.unwrap_or(message.id))
        .bind(message.hops)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Events of one type for one member since a time (wakes in the last hour).
    pub async fn count_room_member_events_since(
        &self,
        room_id: RoomId,
        member_id: RoomMemberId,
        event_type: &str,
        since: DateTime<Utc>,
    ) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM room_events
            WHERE room_id = $1 AND member_id = $2 AND event_type = $3 AND created_at >= $4
            "#,
        )
        .bind(room_id)
        .bind(member_id)
        .bind(event_type)
        .bind(since)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn find_room_agent_session_by_session(
        &self,
        session_id: crate::kernel_imports::contracts::typed_id::SessionId,
    ) -> Result<Option<RoomAgentSessionRow>> {
        Ok(sqlx::query_as::<_, RoomAgentSessionRow>(sql!(
            "SELECT {RoomAgentSessionRow} FROM room_agent_sessions WHERE session_id = $1"
        ))
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_room_agent_sessions(
        &self,
        room_id: RoomId,
    ) -> Result<Vec<RoomAgentSessionRow>> {
        Ok(sqlx::query_as::<_, RoomAgentSessionRow>(sql!(
            r#"
            SELECT {RoomAgentSessionRow} FROM room_agent_sessions
            WHERE room_id = $1
            ORDER BY created_at
            "#
        ))
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

fn message_event(row: &RoomMessageRow, event_type: &str) -> CreateRoomEventRow {
    CreateRoomEventRow {
        room_id: row.room_id,
        org_id: row.org_id,
        event_type: event_type.into(),
        message_id: Some(row.id),
        member_id: row.author_member_id,
        data: serde_json::json!({ "thread_id": row.thread_id }),
    }
}

/// Append a room event inside the caller's transaction.
///
/// Decision: the org's room events are serialized by a transaction-scoped
/// advisory lock taken just before the insert (the last step of every writer),
/// so sequence numbers commit in order and a reader polling `seq > n` never
/// skips an event that commits late with a lower number.
async fn insert_event(
    conn: &mut sqlx::PgConnection,
    input: CreateRoomEventRow,
) -> Result<RoomEventRow> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('room_events:' || $1::text, 0))")
        .bind(input.org_id)
        .execute(&mut *conn)
        .await?;
    Ok(sqlx::query_as::<_, RoomEventRow>(sql!(
        r#"
        INSERT INTO room_events (room_id, org_id, event_type, message_id, member_id, data)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING {RoomEventRow}
        "#
    ))
    .bind(input.room_id)
    .bind(input.org_id)
    .bind(&input.event_type)
    .bind(input.message_id)
    .bind(input.member_id)
    .bind(&input.data)
    .fetch_one(&mut *conn)
    .await?)
}
