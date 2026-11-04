use crate::{
    CacheStats, EventQuery, EventRecord, Limits, QueryPage, ReadProfile, Scope, catalog, query,
    segment,
};
use anyhow::{Context, Result, ensure};
use futures::TryStreamExt;
use object_store::{ObjectStore, ObjectStoreExt, path::Path};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

/// An operator-owned store. Do not expose this API without authenticated Scope
/// construction; possession of a PostgreSQL pool is already privileged access.
#[derive(Clone)]
pub struct TraceStore {
    pub(crate) pool: PgPool,
    objects: Arc<dyn ObjectStore>,
    prefix: Path,
    pub(crate) name: String,
    pub(crate) limits: Limits,
    engine: Arc<query::QueryEngine>,
}

impl TraceStore {
    pub fn new(
        pool: PgPool,
        objects: Arc<dyn ObjectStore>,
        name: impl Into<String>,
        prefix: Path,
        limits: Limits,
    ) -> Result<Self> {
        limits.validate()?;
        let name = name.into();
        ensure!(
            !name.is_empty()
                && name.len() <= 100
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid store name"
        );
        ensure!(
            !prefix.as_ref().is_empty(),
            "object prefix must not be empty"
        );
        let engine = Arc::new(query::QueryEngine::new(&limits)?);
        Ok(Self {
            pool,
            objects,
            prefix,
            name,
            limits,
            engine,
        })
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.engine.cache_stats()
    }

    pub(crate) async fn begin(
        &self,
        exclusive: bool,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = catalog::begin(&self.pool).await?;
        // GC must not race objects uploaded before manifest publication. All
        // readers/writers share this lock; GC alone takes it exclusively.
        let sql = if exclusive {
            "WITH guard AS MATERIALIZED (SELECT pg_advisory_xact_lock(hashtextextended($1,37021))) SELECT l.object_prefix FROM guard LEFT JOIN trace_store_locations l ON l.store_name=$1"
        } else {
            "WITH guard AS MATERIALIZED (SELECT pg_advisory_xact_lock_shared(hashtextextended($1,37021))) SELECT l.object_prefix FROM guard LEFT JOIN trace_store_locations l ON l.store_name=$1"
        };
        let mut prefix: Option<String> = sqlx::query_scalar(sql)
            .bind(&self.name)
            .fetch_one(&mut *tx)
            .await?;
        if prefix.is_none() {
            // Registration is rare. Serialize it across stores so overlapping
            // prefixes cannot make one store's collector delete another's data.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('everruns-trace-store-locations',37022))")
                .execute(&mut *tx).await?;
            prefix = sqlx::query_scalar(
                "SELECT object_prefix FROM trace_store_locations WHERE store_name=$1",
            )
            .bind(&self.name)
            .fetch_optional(&mut *tx)
            .await?;
            if prefix.is_none() {
                let overlaps: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trace_store_locations WHERE object_prefix=$1 OR starts_with(object_prefix,$1 || '/') OR starts_with($1,object_prefix || '/'))")
                    .bind(self.prefix.as_ref()).fetch_one(&mut *tx).await?;
                ensure!(!overlaps, "object prefix overlaps an existing trace store");
                sqlx::query(
                    "INSERT INTO trace_store_locations (store_name,object_prefix) VALUES ($1,$2)",
                )
                .bind(&self.name)
                .bind(self.prefix.as_ref())
                .execute(&mut *tx)
                .await?;
                prefix = Some(self.prefix.to_string());
            }
        }
        ensure!(
            prefix.as_deref() == Some(self.prefix.as_ref()),
            "store name is already bound to a different object prefix"
        );
        Ok(tx)
    }

    /// Export one bounded batch from the canonical source. Repeating this call
    /// resumes the committed sequence; a crash before publication advances nothing.
    pub async fn export_session(&self, scope: Scope, session: Uuid) -> Result<usize> {
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.export_inner(scope, session),
        )
        .await
        .context("export deadline exceeded")?
    }

    async fn export_inner(&self, scope: Scope, session: Uuid) -> Result<usize> {
        self.export_through(scope, session, None).await
    }

    pub(crate) async fn export_through(
        &self,
        scope: Scope,
        session: Uuid,
        target: Option<i32>,
    ) -> Result<usize> {
        let mut tx = self.begin(false).await?;
        catalog::lock_source(&mut tx, scope, session).await?;
        let after = catalog::lock_checkpoint(&mut tx, &self.name, scope, session).await?;
        let head: i32 = sqlx::query_scalar(
            "SELECT COALESCE((SELECT next_sequence-1 FROM event_sequences WHERE session_id=$1),0)",
        )
        .bind(session)
        .fetch_one(&mut *tx)
        .await?;
        let through = target.unwrap_or(head).min(head);
        if after >= through {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(0);
        }
        let batch = catalog::source_batch(&mut tx, session, after, through, &self.limits).await?;
        if !batch.events.is_empty() {
            let manifest = segment::write(
                &self.objects,
                &self.prefix,
                scope,
                &batch.events,
                &self.limits,
            )
            .await?;
            catalog::publish(&mut tx, &self.name, scope, &manifest).await?;
        }
        sqlx::query("UPDATE trace_store_sessions SET exported_sequence=$4,updated_at=now() WHERE store_name=$1 AND org_id=$2 AND session_id=$3")
            .bind(&self.name).bind(scope.org_id()).bind(session).bind(batch.scanned_through).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(batch.events.len())
    }

    /// Fair, bounded discovery for the standalone sync loop. Repeating a pass
    /// eventually drains each session without letting one busy session starve it.
    pub async fn sync_once(&self, scope: Scope, max_sessions: usize) -> Result<usize> {
        ensure!(
            (1..=100).contains(&max_sessions),
            "session batch must be 1..=100"
        );
        let mut discovery = catalog::begin(&self.pool).await?;
        let sessions: Vec<Uuid> = sqlx::query_scalar("SELECT s.id FROM sessions s JOIN event_sequences q ON q.session_id=s.id LEFT JOIN trace_store_sessions c ON c.session_id=s.id AND c.org_id=s.org_id AND c.store_name=$1 WHERE s.org_id=$2 AND NOT COALESCE(c.deleted,FALSE) AND q.next_sequence-1>COALESCE(c.exported_sequence,0) ORDER BY c.updated_at ASC NULLS FIRST,s.id LIMIT $3")
            .bind(&self.name).bind(scope.org_id()).bind(i64::try_from(max_sessions)?).fetch_all(&mut *discovery).await?;
        everruns_durable::commit_transaction(discovery).await?;
        let mut exported = 0;
        for session in sessions {
            exported += self.export_session(scope, session).await?;
        }
        Ok(exported)
    }

    /// Queries the exported snapshot only. A budget overflow is an explicit error,
    /// never a silently partial result. No bucket listing or raw SQL is accepted.
    pub async fn query(&self, scope: Scope, query: &EventQuery) -> Result<QueryPage> {
        self.query_mode(scope, query, false).await
    }

    /// Equivalent-query analytical probe; it cannot skip catalog admission.
    pub(crate) async fn query_datafusion(
        &self,
        scope: Scope,
        query: &EventQuery,
    ) -> Result<QueryPage> {
        self.query_mode(scope, query, true).await
    }

    async fn query_mode(
        &self,
        scope: Scope,
        query: &EventQuery,
        force_datafusion: bool,
    ) -> Result<QueryPage> {
        query.validate()?;
        tokio::time::timeout(self.limits.query_timeout, async {
            let started = std::time::Instant::now();
            let mut profile = ReadProfile::default();
            let mut tx = self.begin(false).await?;
            let segments = catalog::select_segments(
                &mut tx,
                &self.name,
                scope,
                query,
                self.limits.max_query_segments,
            )
            .await?;
            profile.catalog_seconds = started.elapsed().as_secs_f64();
            let mut hits = self
                .engine
                .execute(
                    self.objects.clone(),
                    &self.prefix,
                    scope,
                    &segments,
                    query,
                    &self.limits,
                    None,
                    force_datafusion,
                    &mut profile,
                )
                .await?;
            let has_more = hits.len() > query.limit;
            hits.truncate(query.limit);
            let commit = std::time::Instant::now();
            everruns_durable::commit_transaction(tx).await?;
            profile.catalog_seconds += commit.elapsed().as_secs_f64();
            profile.total_seconds = started.elapsed().as_secs_f64();
            Ok(QueryPage {
                events: hits.into_iter().map(|h| h.summary).collect(),
                has_more,
                segments_scanned: profile.segments_scanned,
                profile,
            })
        })
        .await
        .context("query deadline exceeded")?
    }

    /// Resolve the payload through a scoped manifest, never a caller-provided key.
    pub async fn read_event(
        &self,
        scope: Scope,
        session: Uuid,
        id: Uuid,
    ) -> Result<Option<EventRecord>> {
        self.read_event_range(scope, session, id, None).await
    }

    /// A known canonical sequence prunes detail reads to its containing segment.
    pub async fn read_event_at(
        &self,
        scope: Scope,
        session: Uuid,
        id: Uuid,
        sequence: i32,
    ) -> Result<Option<EventRecord>> {
        ensure!(sequence > 0, "event sequence must be positive");
        self.read_event_range(scope, session, id, Some(sequence))
            .await
    }

    async fn read_event_range(
        &self,
        scope: Scope,
        session: Uuid,
        id: Uuid,
        sequence: Option<i32>,
    ) -> Result<Option<EventRecord>> {
        self.read_event_profiled(scope, session, id, sequence)
            .await
            .map(|(record, _)| record)
    }

    pub(crate) async fn read_event_profiled(
        &self,
        scope: Scope,
        session: Uuid,
        id: Uuid,
        sequence: Option<i32>,
    ) -> Result<(Option<EventRecord>, ReadProfile)> {
        tokio::time::timeout(self.limits.query_timeout, async {
            let started = std::time::Instant::now();
            let mut profile = ReadProfile::default();
            let query = EventQuery {
                session_id: Some(session),
                after_sequence: sequence.map(|s| s - 1),
                through_sequence: sequence,
                limit: 1,
                ..Default::default()
            };
            let mut tx = self.begin(false).await?;
            let segments = catalog::select_segments(
                &mut tx,
                &self.name,
                scope,
                &query,
                self.limits.max_query_segments,
            )
            .await?;
            profile.catalog_seconds = started.elapsed().as_secs_f64();
            let hit = self
                .engine
                .lookup(
                    self.objects.clone(),
                    &self.prefix,
                    scope,
                    &segments,
                    id,
                    sequence,
                    &self.limits,
                    &mut profile,
                )
                .await?;
            let Some(hit) = hit else {
                let commit = std::time::Instant::now();
                everruns_durable::commit_transaction(tx).await?;
                profile.catalog_seconds += commit.elapsed().as_secs_f64();
                profile.total_seconds = started.elapsed().as_secs_f64();
                return Ok((None, profile));
            };
            ensure!(
                hit.length <= self.limits.max_event_bytes as u64,
                "payload byte budget exceeded"
            );
            let segment = segments
                .iter()
                .find(|s| s.payload_key == hit.payload_key)
                .context("payload references an unselected segment")?;
            let end = hit
                .offset
                .checked_add(hit.length)
                .context("payload range overflow")?;
            ensure!(
                end <= u64::try_from(segment.payload_bytes)?,
                "payload outside segment bounds"
            );
            let payload_start = std::time::Instant::now();
            let bytes = self
                .objects
                .get_range(&Path::from(hit.payload_key.as_str()), hit.offset..end)
                .await?;
            profile.payload_io_seconds = payload_start.elapsed().as_secs_f64();
            let decode = std::time::Instant::now();
            ensure!(
                segment::sha256(&bytes) == hit.sha256,
                "event payload checksum mismatch"
            );
            let record: EventRecord = serde_json::from_slice(&bytes)?;
            ensure!(
                record.id == id
                    && record.session_id == session
                    && record.sequence == hit.summary.sequence,
                "payload identity mismatch"
            );
            profile.payload_decode_seconds = decode.elapsed().as_secs_f64();
            let commit = std::time::Instant::now();
            everruns_durable::commit_transaction(tx).await?;
            profile.catalog_seconds += commit.elapsed().as_secs_f64();
            profile.total_seconds = started.elapsed().as_secs_f64();
            Ok((Some(record), profile))
        })
        .await
        .context("payload read deadline exceeded")?
    }

    /// Merge a bounded adjacent group of one session's live segments. Publishing the new
    /// manifest and retiring all inputs happen atomically under its checkpoint.
    pub async fn compact_session(&self, scope: Scope, session: Uuid) -> Result<usize> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut tx = self.begin(false).await?;
            catalog::lock_source(&mut tx, scope, session).await?;
            catalog::lock_checkpoint(&mut tx, &self.name, scope, session).await?;
            let q = EventQuery {
                session_id: Some(session),
                limit: 1000,
                ..Default::default()
            };
            let candidates = catalog::select_segments(
                &mut tx,
                &self.name,
                scope,
                &q,
                self.limits.max_query_segments,
            )
            .await?;
            let mut inputs = Vec::new();
            let mut rows = 0_i64;
            let mut bytes = 0_i64;
            let max_rows = i64::try_from(self.limits.max_compaction_rows)?;
            let max_bytes = i64::try_from(self.limits.max_compaction_bytes)?;
            for s in candidates {
                if rows + s.row_count > max_rows || bytes + s.payload_bytes > max_bytes {
                    if inputs.len() >= 2 {
                        break;
                    }
                    // A previously compacted large segment must not block all
                    // subsequent small segments from being compacted together.
                    inputs.clear();
                    rows = 0;
                    bytes = 0;
                }
                if s.row_count > max_rows || s.payload_bytes > max_bytes {
                    continue;
                }
                rows += s.row_count;
                bytes += s.payload_bytes;
                inputs.push(s);
            }
            if inputs.len() < 2 {
                everruns_durable::commit_transaction(tx).await?;
                return Ok(0);
            }
            let mut events = BTreeMap::new();
            for input in &inputs {
                ensure!(input.format_version == 1, "unsupported segment format");
                let path = Path::from(input.payload_key.as_str());
                let expected = u64::try_from(input.payload_bytes)?;
                ensure!(
                    self.objects.head(&path).await?.size == expected,
                    "segment byte length mismatch"
                );
                let raw = self.objects.get_range(&path, 0..expected).await?;
                ensure!(
                    i64::try_from(raw.len())? == input.payload_bytes
                        && segment::sha256(&raw) == input.payload_sha256,
                    "segment checksum mismatch"
                );
                let mut count = 0;
                for line in raw.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                    let e: EventRecord = serde_json::from_slice(line)?;
                    ensure!(
                        e.session_id == session
                            && e.sequence >= input.first_sequence
                            && e.sequence <= input.last_sequence,
                        "compaction payload scope mismatch"
                    );
                    ensure!(
                        !events.contains_key(&e.sequence),
                        "overlapping canonical segment sequences"
                    );
                    events.insert(e.sequence, e);
                    count += 1;
                }
                ensure!(count == input.row_count, "segment event count mismatch");
            }
            let records = events.into_values().collect::<Vec<_>>();
            let mut output_limits = self.limits.clone();
            output_limits.max_batch_rows = self.limits.max_compaction_rows;
            output_limits.max_batch_bytes = self.limits.max_compaction_bytes;
            let output =
                segment::write(&self.objects, &self.prefix, scope, &records, &output_limits)
                    .await?;
            catalog::publish(&mut tx, &self.name, scope, &output).await?;
            let ids = inputs.iter().map(|s| s.id).collect::<Vec<_>>();
            sqlx::query("UPDATE trace_store_segments SET retired=TRUE WHERE id=ANY($1)")
                .bind(&ids)
                .execute(&mut *tx)
                .await?;
            everruns_durable::commit_transaction(tx).await?;
            Ok(inputs.len())
        })
        .await
        .context("compaction deadline exceeded")?
    }

    /// Hide exported traces immediately and durably prevent their re-export.
    /// Canonical source events remain untouched; collection removes physical files.
    pub async fn delete_session(&self, scope: Scope, session: Uuid) -> Result<()> {
        let mut tx = self.begin(false).await?;
        // A wrong-tenant deletion must not create a tombstone for another org.
        catalog::lock_source(&mut tx, scope, session).await?;
        sqlx::query("INSERT INTO trace_store_sessions (store_name,org_id,session_id,deleted) VALUES ($1,$2,$3,TRUE) ON CONFLICT (store_name,org_id,session_id) DO UPDATE SET deleted=TRUE")
            .bind(&self.name).bind(scope.org_id()).bind(session).execute(&mut *tx).await?;
        sqlx::query("UPDATE trace_store_segments SET retired=TRUE WHERE store_name=$1 AND org_id=$2 AND session_id=$3")
            .bind(&self.name).bind(scope.org_id()).bind(session).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        self.engine.evict_session(scope, session);
        Ok(())
    }

    /// Bounded physical cleanup, including uploads whose publication crashed.
    /// Continue with the returned object key. GC serializes against readers and
    /// uploaders, so even an unpublished in-flight object cannot be removed.
    pub async fn collect_garbage(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(usize, Option<String>)> {
        ensure!((1..=1000).contains(&limit), "GC limit must be 1..=1000");
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut tx = self.begin(true).await?;
            if let Some(after) = after {
                ensure!(Path::parse(after)?.prefix_matches(&self.prefix), "GC cursor outside store prefix");
            }
            // ObjectStore does not promise listing order. Keep a bounded sorted
            // window, so a resume cursor cannot skip out-of-order objects.
            let mut listed = self.objects.list(Some(&self.prefix));
            let mut window = BTreeMap::new();
            let mut examined = 0;
            while let Some(object) = listed.try_next().await? {
                examined += 1;
                ensure!(examined <= 100_000, "GC listing budget exceeded");
                let key = object.location.to_string();
                if after.is_some_and(|after| key.as_str() <= after) { continue; }
                window.insert(key,object);
                if window.len() > limit + 1 { window.pop_last(); }
            }
            let more = window.len() > limit;
            if more { window.pop_last(); }
            let mut deleted = 0;
            let mut last = None;
            for (key, object) in window {
                let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trace_store_segments m JOIN trace_store_sessions c USING(store_name,org_id,session_id) JOIN sessions s ON s.id=m.session_id AND s.org_id=m.org_id WHERE m.store_name=$1 AND NOT m.retired AND NOT c.deleted AND (m.index_key=$2 OR m.payload_key=$2))")
                    .bind(&self.name).bind(&key).fetch_one(&mut *tx).await?;
                if !live { self.objects.delete(&object.location).await?; deleted += 1; }
                last = Some(key);
            }
            everruns_durable::commit_transaction(tx).await?;
            Ok((deleted, if more { last } else { None }))
        }).await.context("garbage collection deadline exceeded")?
    }
}
