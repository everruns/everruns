use crate::{
    EventQuery, Scope,
    segment::{SEGMENT_COLUMNS, Segment},
};
use anyhow::{Context, Result, ensure};
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

/// Lock order: live source session, then trace checkpoint. Source deletion waits
/// for an in-flight export/read; a subsequent request cannot discover its files.
pub(crate) async fn lock_source(
    tx: &mut Transaction<'_, Postgres>,
    scope: Scope,
    session: Uuid,
) -> Result<()> {
    let live: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM sessions WHERE org_id = $1 AND id = $2 FOR KEY SHARE")
            .bind(scope.org_id())
            .bind(session)
            .fetch_optional(&mut **tx)
            .await?;
    ensure!(live.is_some(), "session not found in organization");
    Ok(())
}

pub(crate) async fn lock_checkpoint(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    scope: Scope,
    session: Uuid,
) -> Result<i32> {
    sqlx::query("INSERT INTO trace_store_sessions (store_name, org_id, session_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(name).bind(scope.org_id()).bind(session).execute(&mut **tx).await?;
    let (sequence, deleted): (i32, bool) = sqlx::query_as("SELECT exported_sequence, deleted FROM trace_store_sessions WHERE store_name=$1 AND org_id=$2 AND session_id=$3 FOR UPDATE")
        .bind(name).bind(scope.org_id()).bind(session).fetch_one(&mut **tx).await?;
    ensure!(!deleted, "session traces have been deleted");
    Ok(sequence)
}

pub(crate) async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    scope: Scope,
    segment: &Segment,
) -> Result<()> {
    sqlx::query("INSERT INTO trace_store_segments (id, store_name, org_id, session_id, first_sequence, last_sequence, min_ts, max_ts, row_count, index_key, payload_key, index_bytes, payload_bytes, index_sha256, payload_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
        .bind(segment.id).bind(name).bind(scope.org_id()).bind(segment.session_id)
        .bind(segment.first_sequence).bind(segment.last_sequence).bind(segment.min_ts).bind(segment.max_ts)
        .bind(segment.row_count).bind(&segment.index_key).bind(&segment.payload_key)
        .bind(segment.index_bytes).bind(segment.payload_bytes).bind(&segment.index_sha256).bind(&segment.payload_sha256)
        .execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn select_segments(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    scope: Scope,
    query: &EventQuery,
    max: usize,
) -> Result<Vec<Segment>> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!(
        "SELECT {SEGMENT_COLUMNS} FROM trace_store_segments m JOIN trace_store_sessions c USING (store_name,org_id,session_id) JOIN sessions s ON s.id=m.session_id AND s.org_id=m.org_id WHERE NOT m.retired AND NOT c.deleted AND m.store_name="
    ));
    qb.push_bind(name)
        .push(" AND m.org_id=")
        .push_bind(scope.org_id());
    if let Some(id) = query.session_id {
        qb.push(" AND m.session_id=").push_bind(id);
    }
    if let Some(seq) = query.after_sequence {
        qb.push(" AND m.last_sequence>").push_bind(seq);
    }
    if let Some(seq) = query.through_sequence {
        qb.push(" AND m.first_sequence<=").push_bind(seq);
    }
    if let Some(from) = query.from {
        qb.push(" AND m.max_ts>=").push_bind(from);
    }
    if let Some(until) = query.until {
        qb.push(" AND m.min_ts<").push_bind(until);
    }
    qb.push(" ORDER BY m.session_id, m.first_sequence LIMIT ")
        .push_bind(i64::try_from(max + 1)?);
    // Hold source and manifest locks until the object reads complete. Compaction
    // and trace deletion cannot retire files underneath this snapshot.
    qb.push(" FOR SHARE OF m, c, s");
    let segments = qb.build_query_as().fetch_all(&mut **tx).await?;
    Ok(segments)
}

pub(crate) async fn begin(pool: &PgPool) -> Result<Transaction<'static, Postgres>> {
    let mut tx = everruns_durable::begin_transaction(pool).await?;
    sqlx::raw_sql("SET LOCAL lock_timeout = '5s'; SET LOCAL statement_timeout = '20s'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

pub(crate) struct SourceBatch {
    pub events: Vec<crate::EventRecord>,
    pub scanned_through: i32,
}

/// Live and archived history form one retained log. Read narrow admission keys
/// first; an archival transaction moving a row between tables cannot lose it
/// because each union statement uses a single PostgreSQL snapshot.
pub(crate) async fn source_batch(
    tx: &mut Transaction<'_, Postgres>,
    session: Uuid,
    after: i32,
    through: i32,
    limits: &crate::Limits,
) -> Result<SourceBatch> {
    let candidates: Vec<(Uuid,i32,i64)>=sqlx::query_as(
        r#"(SELECT id,sequence,(octet_length(data::text)::bigint + octet_length(context::text) + COALESCE(octet_length(metadata::text),0) + COALESCE(octet_length(array_to_json(tags)::text),0) + octet_length(event_type) + 512) AS estimated_bytes FROM events WHERE session_id=$1 AND sequence>$2 AND sequence<=$3 ORDER BY sequence LIMIT $4)
        UNION ALL
        (SELECT id,sequence,(octet_length(data::text)::bigint + octet_length(context::text) + COALESCE(octet_length(metadata::text),0) + COALESCE(octet_length(array_to_json(tags)::text),0) + octet_length(event_type) + 512) AS estimated_bytes FROM archived_events WHERE session_id=$1 AND sequence>$2 AND sequence<=$3 ORDER BY sequence LIMIT $4)"#)
        .bind(session).bind(after).bind(through).bind(i64::try_from(limits.max_batch_rows+1)?).fetch_all(&mut **tx).await?;
    let mut unique = std::collections::BTreeMap::new();
    for (id, sequence, estimate) in candidates {
        if let Some((existing, _)) = unique.insert(sequence, (id, estimate)) {
            ensure!(
                existing == id,
                "conflicting source event identities at one sequence"
            );
        }
    }
    let available = unique.len();
    let mut ids = Vec::new();
    let mut bytes = 0_usize;
    let mut last = after;
    for (sequence, (id, estimate)) in unique.into_iter().take(limits.max_batch_rows) {
        let estimate = usize::try_from(estimate)?;
        ensure!(
            estimate <= limits.max_event_bytes,
            "source event exceeds payload budget"
        );
        let next = bytes
            .checked_add(estimate + 1)
            .context("payload size overflow")?;
        if next > limits.max_batch_bytes {
            break;
        }
        ids.push(id);
        bytes = next;
        last = sequence;
    }
    let scanned_through = if ids.len() == available {
        through
    } else {
        last
    };
    if ids.is_empty() {
        return Ok(SourceBatch {
            events: Vec::new(),
            scanned_through,
        });
    }
    let sql = "SELECT DISTINCT ON(sequence) id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags FROM (SELECT id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags,0 AS priority FROM events WHERE session_id=$1 AND id=ANY($2) UNION ALL SELECT id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags,1 AS priority FROM archived_events WHERE session_id=$1 AND id=ANY($2)) retained ORDER BY sequence,priority";
    let events = sqlx::query_as(sql)
        .bind(session)
        .bind(&ids)
        .fetch_all(&mut **tx)
        .await?;
    ensure!(
        events.len() == ids.len(),
        "source events disappeared during export; checkpoint unchanged"
    );
    Ok(SourceBatch {
        events,
        scanned_through,
    })
}
