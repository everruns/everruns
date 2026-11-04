use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A database tenant id, supplied by an authenticated host or trusted operator.
#[derive(Clone, Copy, Debug)]
pub struct Scope {
    org_id: i64,
}

impl Scope {
    pub fn new(org_id: i64) -> Result<Self> {
        ensure!(org_id > 0, "org_id must be positive");
        Ok(Self { org_id })
    }

    pub fn org_id(self) -> i64 {
        self.org_id
    }
}

/// Lossless persisted envelope. Unknown event types and JSON fields survive export.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, sqlx::FromRow)]
pub struct EventRecord {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub context: serde_json::Value,
    pub data: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
}

/// All limits are enforced by the store, including calls made without the CLI.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_batch_rows: usize,
    pub max_batch_bytes: usize,
    pub max_event_bytes: usize,
    pub max_compaction_rows: usize,
    pub max_compaction_bytes: usize,
    pub max_query_segments: usize,
    pub max_query_index_bytes: u64,
    pub query_memory_bytes: usize,
    /// Decoded immutable indexes only. Zero disables caching for cold probes.
    pub index_cache_bytes: usize,
    pub query_timeout: std::time::Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_batch_rows: 2_000,
            max_batch_bytes: 32 * 1024 * 1024,
            max_event_bytes: 8 * 1024 * 1024,
            max_compaction_rows: 20_000,
            max_compaction_bytes: 128 * 1024 * 1024,
            max_query_segments: 256,
            max_query_index_bytes: 64 * 1024 * 1024,
            query_memory_bytes: 128 * 1024 * 1024,
            index_cache_bytes: 64 * 1024 * 1024,
            query_timeout: std::time::Duration::from_secs(15),
        }
    }
}

impl Limits {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            (1..=10_000).contains(&self.max_batch_rows),
            "invalid batch row limit"
        );
        ensure!(
            self.max_event_bytes > 0 && self.max_event_bytes < self.max_batch_bytes,
            "invalid payload limits"
        );
        ensure!(
            (1..=100_000).contains(&self.max_compaction_rows)
                && self.max_compaction_bytes >= self.max_event_bytes,
            "invalid compaction budgets"
        );
        ensure!(
            (1..=1024).contains(&self.max_query_segments),
            "invalid segment limit"
        );
        ensure!(
            self.max_query_index_bytes > 0
                && self.query_memory_bytes > 0
                && !self.query_timeout.is_zero(),
            "query budgets must be positive"
        );
        Ok(())
    }
}

/// Typed predicates only: callers cannot submit SQL or choose object paths.
#[derive(Clone, Debug)]
pub struct EventQuery {
    pub session_id: Option<Uuid>,
    pub after_sequence: Option<i32>,
    /// Inclusive upper bound, used to compare the same immutable prefix.
    pub through_sequence: Option<i32>,
    pub event_types: Vec<String>,
    pub turn_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    /// Results are ordered by session id, then canonical sequence.
    pub limit: usize,
}

impl Default for EventQuery {
    fn default() -> Self {
        Self {
            session_id: None,
            after_sequence: None,
            through_sequence: None,
            event_types: Vec::new(),
            turn_id: None,
            from: None,
            until: None,
            limit: 100,
        }
    }
}

impl EventQuery {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1000).contains(&self.limit),
            "query limit must be 1..=1000"
        );
        ensure!(
            self.after_sequence.is_none() || self.session_id.is_some(),
            "after_sequence requires a session"
        );
        ensure!(
            self.after_sequence.is_none_or(|s| s >= 0),
            "negative sequence"
        );
        ensure!(
            self.through_sequence.is_none() || self.session_id.is_some(),
            "through_sequence requires a session"
        );
        ensure!(
            self.through_sequence.is_none_or(|s| s >= 0)
                && self
                    .after_sequence
                    .zip(self.through_sequence)
                    .is_none_or(|(a, b)| a <= b),
            "invalid sequence range"
        );
        ensure!(
            self.event_types.len() <= 32 && self.event_types.iter().all(|t| t.len() <= 100),
            "too many or oversized event type filters"
        );
        ensure!(
            self.turn_id.as_ref().is_none_or(|s| s.len() <= 128),
            "turn id too long"
        );
        ensure!(
            self.from.zip(self.until).is_none_or(|(a, b)| a < b),
            "invalid time range"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, sqlx::FromRow)]
pub struct EventSummary {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub turn_id: Option<String>,
    pub exec_id: Option<String>,
    pub parent_exec_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QueryPage {
    pub events: Vec<EventSummary>,
    pub has_more: bool,
    pub segments_scanned: usize,
    pub profile: ReadProfile,
}

/// Fixed stages only; contains no event identities, object keys or payloads.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadProfile {
    pub engine: ReadEngine,
    pub segments_scanned: usize,
    pub total_seconds: f64,
    pub catalog_seconds: f64,
    pub index_io_seconds: f64,
    pub index_decode_seconds: f64,
    pub planning_seconds: f64,
    pub execution_seconds: f64,
    pub payload_io_seconds: f64,
    pub payload_decode_seconds: f64,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub index_bytes_read: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadEngine {
    #[default]
    None,
    SessionIndex,
    Datafusion,
    Locator,
}
impl ReadEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SessionIndex => "session_index",
            Self::Datafusion => "datafusion",
            Self::Locator => "locator",
        }
    }
}

impl ReadProfile {
    pub fn cache_state(&self) -> &'static str {
        match (self.cache_hits > 0, self.cache_misses > 0) {
            (true, false) => "warm",
            (false, true) => "cold",
            (true, true) => "mixed",
            (false, false) => "empty",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CacheStats {
    pub bytes: usize,
    pub entries: usize,
    pub evictions: u64,
}
