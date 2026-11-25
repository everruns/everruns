// PostgreSQL repository: room memory (knowledge/runtime-resources/rooms.md,
// "Room memory").
//
// Decisions:
// - Which fact replaces which is recomputed for the whole subject whenever a
//   fact of it is recorded or forgotten, under a per-room advisory lock. A
//   subject holds a handful of facts, and recomputing makes a backdated fact
//   land in the right place: it replaces what came before it and is itself
//   replaced by what came after.
// - `superseded_at` is when the replacing fact happened, not when it was
//   recorded, so "what was true on a date" is `superseded_at IS NULL OR
//   superseded_at > date`.
// - Nothing is purged. A note re-sent under the same caller id with new
//   text, label or metadata hides the old row and adds a new one; forgetting sets `hidden_at`.
// - Recall reads messages and entries with the same stemmed `english`
//   configuration the indexes were built with; the query string is built by
//   the domain from plain words only (`rooms::memory::recall_query`).

use super::rows::*;
use super::{Database, insert_event};
use crate::kernel_imports::contracts::typed_id::{
    RoomId, RoomMemberId, RoomMemoryEntryId, RoomMessageId, SessionId,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_server_macros::sql;
use sqlx::FromRow;

/// Fact kinds where a newer fact on the same subject replaces the older one.
/// Questions and observations stand on their own.
pub const REPLACING_FACT_KINDS: &[&str] = &["decision", "rule", "claim", "deadline"];

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RoomMemoryEntryRow {
    pub id: RoomMemoryEntryId,
    pub room_id: RoomId,
    pub org_id: i64,
    pub entry_kind: String,
    pub fact_kind: Option<String>,
    pub subject: Option<String>,
    pub statement: String,
    pub quote: Option<String>,
    pub status: String,
    pub occurred_at: DateTime<Utc>,
    pub author_member_id: Option<RoomMemberId>,
    pub source_kind: String,
    pub source_message_id: Option<RoomMessageId>,
    pub source_session_id: Option<SessionId>,
    pub source_ref: Option<String>,
    pub label: Option<String>,
    pub metadata: serde_json::Value,
    pub recorded_by: String,
    pub superseded_at: Option<DateTime<Utc>>,
    pub superseded_by: Option<RoomMemoryEntryId>,
    pub hidden_at: Option<DateTime<Utc>>,
    pub hidden_by_member_id: Option<RoomMemberId>,
    pub created_at: DateTime<Utc>,
}

/// A fact or a note to record.
#[derive(Debug, Clone)]
pub struct CreateRoomMemoryEntryRow {
    pub room_id: RoomId,
    pub org_id: i64,
    pub entry_kind: String,
    pub fact_kind: Option<String>,
    pub subject: Option<String>,
    pub statement: String,
    pub quote: Option<String>,
    pub status: String,
    pub occurred_at: DateTime<Utc>,
    pub author_member_id: Option<RoomMemberId>,
    pub source_kind: String,
    pub source_message_id: Option<RoomMessageId>,
    pub source_session_id: Option<SessionId>,
    pub source_ref: Option<String>,
    pub label: Option<String>,
    pub metadata: serde_json::Value,
    pub recorded_by: String,
}

/// What recording an entry did.
#[derive(Debug, Clone)]
pub enum RoomMemoryWrite {
    /// Recorded. `replaced` are the facts it now replaces.
    Recorded {
        entry: RoomMemoryEntryRow,
        replaced: Vec<RoomMemoryEntryId>,
    },
    /// The same caller id with the same content: nothing changed.
    Unchanged(RoomMemoryEntryRow),
}

/// A message recall found, with how well it matched and the matching words.
#[derive(Debug, Clone, FromRow)]
pub struct RecalledRoomMessageRow {
    #[sqlx(flatten)]
    pub message: RoomMessageRow,
    pub rank: f32,
    pub excerpt: String,
}

/// An entry recall found, with how well it matched.
#[derive(Debug, Clone, FromRow)]
pub struct RecalledRoomEntryRow {
    #[sqlx(flatten)]
    pub entry: RoomMemoryEntryRow,
    pub rank: f32,
}

/// Which facts to list.
#[derive(Debug, Clone, Default)]
pub struct RoomFactFilter {
    pub fact_kind: Option<String>,
    pub subject: Option<String>,
    /// Read as of this time: later facts are out, and a fact replaced after
    /// it still counts as current.
    pub as_of: Option<DateTime<Utc>>,
    /// Also list facts that were replaced.
    pub include_replaced: bool,
    pub limit: i64,
}

async fn lock_room_memory(conn: &mut sqlx::PgConnection, room_id: RoomId) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('room_memory:' || $1::text, 0))")
        .bind(room_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn insert_entry(
    conn: &mut sqlx::PgConnection,
    input: &CreateRoomMemoryEntryRow,
) -> Result<RoomMemoryEntryRow> {
    Ok(sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
        r#"
        INSERT INTO room_memory_entries (room_id, org_id, entry_kind, fact_kind, subject, statement,
                                         quote, status, occurred_at, author_member_id, source_kind,
                                         source_message_id, source_session_id, source_ref, label,
                                         metadata, recorded_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        RETURNING {RoomMemoryEntryRow}
        "#
    ))
    .bind(input.room_id)
    .bind(input.org_id)
    .bind(&input.entry_kind)
    .bind(&input.fact_kind)
    .bind(&input.subject)
    .bind(&input.statement)
    .bind(&input.quote)
    .bind(&input.status)
    .bind(input.occurred_at)
    .bind(input.author_member_id)
    .bind(&input.source_kind)
    .bind(input.source_message_id)
    .bind(input.source_session_id)
    .bind(&input.source_ref)
    .bind(&input.label)
    .bind(&input.metadata)
    .bind(&input.recorded_by)
    .fetch_one(&mut *conn)
    .await?)
}

/// Whether a fact of this status replaces earlier ones. A proposal or a
/// guess never replaces anything; a retraction does.
pub fn status_replaces(status: &str) -> bool {
    matches!(status, "stated" | "retracted")
}

/// Which entry replaces which, for one subject's live facts in time order:
/// each replacing fact replaces everything still current before it.
/// Returns, per input index, the index of the fact that replaces it.
pub fn plan_replacements(facts: &[(DateTime<Utc>, &str)]) -> Vec<Option<usize>> {
    let mut out = vec![None; facts.len()];
    let mut pending: Vec<usize> = Vec::new();
    for (index, (_, status)) in facts.iter().enumerate() {
        if status_replaces(status) {
            for earlier in pending.drain(..) {
                out[earlier] = Some(index);
            }
        }
        pending.push(index);
    }
    out
}

/// Recompute which fact replaces which for one subject. Returns the ids of
/// facts whose replacement changed to `by`.
async fn recompute_subject(
    conn: &mut sqlx::PgConnection,
    room_id: RoomId,
    fact_kind: &str,
    subject: &str,
) -> Result<Vec<(RoomMemoryEntryId, Option<RoomMemoryEntryId>)>> {
    if !REPLACING_FACT_KINDS.contains(&fact_kind) {
        return Ok(Vec::new());
    }
    let facts = sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
        r#"
        SELECT {RoomMemoryEntryRow} FROM room_memory_entries
        WHERE room_id = $1 AND entry_kind = 'fact' AND fact_kind = $2 AND subject = $3
          AND hidden_at IS NULL
        ORDER BY occurred_at, created_at, id
        "#
    ))
    .bind(room_id)
    .bind(fact_kind)
    .bind(subject)
    .fetch_all(&mut *conn)
    .await?;
    let plan = plan_replacements(
        &facts
            .iter()
            .map(|fact| (fact.occurred_at, fact.status.as_str()))
            .collect::<Vec<_>>(),
    );
    let mut changed = Vec::new();
    for (fact, by) in facts.iter().zip(plan) {
        let (at, by_id) = match by {
            Some(index) => (Some(facts[index].occurred_at), Some(facts[index].id)),
            None => (None, None),
        };
        if fact.superseded_by != by_id || fact.superseded_at != at {
            sqlx::query(
                "UPDATE room_memory_entries SET superseded_at = $2, superseded_by = $3 WHERE id = $1",
            )
            .bind(fact.id)
            .bind(at)
            .bind(by_id)
            .execute(&mut *conn)
            .await?;
            changed.push((fact.id, by_id));
        }
    }
    Ok(changed)
}

impl Database {
    /// Record a fact or a note. A fact recomputes its subject's replacements;
    /// a note with a caller id replaces the live note of that id when its
    /// content changed.
    pub async fn record_room_memory_entry(
        &self,
        input: CreateRoomMemoryEntryRow,
    ) -> Result<RoomMemoryWrite> {
        let mut tx = self.pool.begin().await?;
        lock_room_memory(&mut tx, input.room_id).await?;
        if let Some(source_ref) = &input.source_ref {
            let live = sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
                r#"
                SELECT {RoomMemoryEntryRow} FROM room_memory_entries
                WHERE room_id = $1 AND source_ref = $2 AND hidden_at IS NULL
                "#
            ))
            .bind(input.room_id)
            .bind(source_ref)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(live) = live {
                // The time is left out: a re-send without one defaults to
                // now, and the same text is the same note.
                if live.statement == input.statement
                    && live.label == input.label
                    && live.metadata == input.metadata
                {
                    tx.commit().await?;
                    return Ok(RoomMemoryWrite::Unchanged(live));
                }
                sqlx::query(
                    "UPDATE room_memory_entries SET hidden_at = now(), hidden_by_member_id = $2 WHERE id = $1",
                )
                .bind(live.id)
                .bind(input.author_member_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        let entry = insert_entry(&mut tx, &input).await?;
        let mut replaced = Vec::new();
        if let (Some(kind), Some(subject)) = (&entry.fact_kind, &entry.subject) {
            for (id, by) in recompute_subject(&mut tx, entry.room_id, kind, subject).await? {
                if by == Some(entry.id) {
                    replaced.push(id);
                }
            }
        }
        // Re-read: recomputing may have marked the new fact itself replaced
        // (a backdated fact older than the current one).
        let entry = sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
            "SELECT {RoomMemoryEntryRow} FROM room_memory_entries WHERE id = $1"
        ))
        .bind(entry.id)
        .fetch_one(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            CreateRoomEventRow {
                room_id: entry.room_id,
                org_id: entry.org_id,
                event_type: "memory.recorded".into(),
                message_id: entry.source_message_id,
                member_id: entry.author_member_id,
                data: serde_json::json!({
                    "entry_id": entry.id.to_string(),
                    "entry_kind": entry.entry_kind,
                    "fact_kind": entry.fact_kind,
                    "recorded_by": entry.recorded_by,
                    "replaced": replaced.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(RoomMemoryWrite::Recorded { entry, replaced })
    }

    pub async fn get_room_memory_entry(
        &self,
        room_id: RoomId,
        entry_id: RoomMemoryEntryId,
    ) -> Result<Option<RoomMemoryEntryRow>> {
        Ok(sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
            "SELECT {RoomMemoryEntryRow} FROM room_memory_entries WHERE room_id = $1 AND id = $2"
        ))
        .bind(room_id)
        .bind(entry_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Every live fact on one subject, oldest first: its history.
    pub async fn list_room_subject_history(
        &self,
        room_id: RoomId,
        fact_kind: &str,
        subject: &str,
    ) -> Result<Vec<RoomMemoryEntryRow>> {
        Ok(sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
            r#"
            SELECT {RoomMemoryEntryRow} FROM room_memory_entries
            WHERE room_id = $1 AND entry_kind = 'fact' AND fact_kind = $2 AND subject = $3
              AND hidden_at IS NULL
            ORDER BY occurred_at, created_at, id
            "#
        ))
        .bind(room_id)
        .bind(fact_kind)
        .bind(subject)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The room's facts, newest first: current ones only unless asked.
    pub async fn list_room_facts(
        &self,
        room_id: RoomId,
        filter: &RoomFactFilter,
    ) -> Result<Vec<RoomMemoryEntryRow>> {
        Ok(sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
            r#"
            SELECT {RoomMemoryEntryRow} FROM room_memory_entries
            WHERE room_id = $1 AND entry_kind = 'fact' AND hidden_at IS NULL
              AND ($2::text IS NULL OR fact_kind = $2)
              AND ($3::text IS NULL OR subject = $3)
              AND occurred_at <= COALESCE($4, now())
              AND ($5 OR superseded_at IS NULL OR superseded_at > COALESCE($4, now()))
            ORDER BY occurred_at DESC, created_at DESC
            LIMIT $6
            "#
        ))
        .bind(room_id)
        .bind(&filter.fact_kind)
        .bind(&filter.subject)
        .bind(filter.as_of)
        .bind(filter.include_replaced)
        .bind(filter.limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Facts recorded from one message, so a message is read once.
    pub async fn count_room_memory_from_message(
        &self,
        room_id: RoomId,
        message_id: RoomMessageId,
    ) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM room_memory_entries WHERE room_id = $1 AND source_message_id = $2 AND recorded_by = 'extraction'",
        )
        .bind(room_id)
        .bind(message_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Take an entry out of recall. A forgotten fact gives its place back:
    /// the fact it replaced is current again. Returns `None` when there is
    /// no live entry with that id.
    pub async fn forget_room_memory_entry(
        &self,
        room_id: RoomId,
        entry_id: RoomMemoryEntryId,
        by: Option<RoomMemberId>,
    ) -> Result<Option<RoomMemoryEntryRow>> {
        let mut tx = self.pool.begin().await?;
        lock_room_memory(&mut tx, room_id).await?;
        let Some(entry) = sqlx::query_as::<_, RoomMemoryEntryRow>(sql!(
            r#"
            UPDATE room_memory_entries SET hidden_at = now(), hidden_by_member_id = $3
            WHERE room_id = $1 AND id = $2 AND hidden_at IS NULL
            RETURNING {RoomMemoryEntryRow}
            "#
        ))
        .bind(room_id)
        .bind(entry_id)
        .bind(by)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        if let (Some(kind), Some(subject)) = (&entry.fact_kind, &entry.subject) {
            recompute_subject(&mut tx, room_id, kind, subject).await?;
        }
        insert_event(
            &mut tx,
            CreateRoomEventRow {
                room_id,
                org_id: entry.org_id,
                event_type: "memory.forgotten".into(),
                message_id: None,
                member_id: by,
                data: serde_json::json!({ "entry_id": entry.id.to_string() }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Some(entry))
    }

    /// Messages whose words match `query` (a `to_tsquery` string), best
    /// first, posted at or before `as_of`.
    pub async fn recall_room_messages(
        &self,
        room_id: RoomId,
        query: &str,
        as_of: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RecalledRoomMessageRow>> {
        let columns = super::message_columns("m");
        let sql = format!(
            r#"
            SELECT {columns},
                   ts_rank_cd(to_tsvector('english', m.text), q.query)::real AS rank,
                   ts_headline('english', m.text, q.query,
                               'MaxWords=40, MinWords=12, MaxFragments=2, FragmentDelimiter=" … ", StartSel=**, StopSel=**') AS excerpt
            FROM room_messages m, to_tsquery('english', $2) AS q(query)
            WHERE m.room_id = $1 AND m.deleted_at IS NULL
              AND to_tsvector('english', m.text) @@ q.query
              AND m.created_at <= $3
            ORDER BY rank DESC, m.created_at DESC
            LIMIT $4
            "#
        );
        // Safe: the only interpolation is the fixed column list.
        Ok(
            sqlx::query_as::<_, RecalledRoomMessageRow>(sqlx::AssertSqlSafe(sql))
                .bind(room_id)
                .bind(query)
                .bind(as_of)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Live entries whose words match `query`, best first, that happened at
    /// or before `as_of`. Replaced facts are included; the caller ranks them.
    pub async fn recall_room_entries(
        &self,
        room_id: RoomId,
        query: &str,
        as_of: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RecalledRoomEntryRow>> {
        let columns = RoomMemoryEntryRow::COLUMNS
            .split(", ")
            .map(|column| format!("e.{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            r#"
            SELECT {columns}, ts_rank_cd(e.terms, q.query)::real AS rank
            FROM room_memory_entries e, to_tsquery('english', $2) AS q(query)
            WHERE e.room_id = $1 AND e.hidden_at IS NULL
              AND e.terms @@ q.query
              AND e.occurred_at <= $3
            ORDER BY rank DESC, e.occurred_at DESC
            LIMIT $4
            "#
        );
        // Safe: the only interpolation is the fixed column list.
        Ok(
            sqlx::query_as::<_, RecalledRoomEntryRow>(sqlx::AssertSqlSafe(sql))
                .bind(room_id)
                .bind(query)
                .bind(as_of)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        format!("2026-03-{day:02}T00:00:00Z").parse().unwrap()
    }

    #[test]
    fn a_newer_stated_fact_replaces_everything_before_it() {
        let plan = plan_replacements(&[(at(1), "stated"), (at(2), "stated"), (at(3), "stated")]);
        assert_eq!(plan, vec![Some(1), Some(2), None]);
    }

    #[test]
    fn a_proposal_never_replaces_but_can_be_replaced() {
        let plan = plan_replacements(&[(at(1), "stated"), (at(2), "proposed"), (at(3), "stated")]);
        // The proposal stays next to the stated fact until a decision lands.
        assert_eq!(plan, vec![Some(2), Some(2), None]);
        let plan = plan_replacements(&[(at(1), "stated"), (at(2), "hypothetical")]);
        assert_eq!(plan, vec![None, None]);
    }

    #[test]
    fn a_retraction_replaces_and_stays_current() {
        let plan = plan_replacements(&[(at(1), "stated"), (at(2), "retracted")]);
        assert_eq!(plan, vec![Some(1), None]);
    }
}
