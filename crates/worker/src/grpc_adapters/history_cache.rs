//! Per-worker cache of session message history.
//!
//! Decision: a reason step used to reload a session's whole history over gRPC,
//! so the server's cost per step grew with the session. History on this path
//! is append-only (it is derived from `input.message`,
//! `output.message.completed` and `tool.completed` events, which are never
//! edited) and one session's events commit in sequence order (the
//! `event_sequences` row lock serializes writers). So the messages after the
//! last cached sequence are exactly what changed, whoever wrote them: a
//! session that moved to another worker and back still reads correctly.
//! Design: `knowledge/execution/worker-history-cache.md`.
//!
//! A "latest N" read caches the window it read and the session's message
//! count, and later windows no wider than what it holds (one new message
//! widens it by one) are served from it the same way. The reason step reads
//! a window on every model call, so before windows were kept the cache never
//! warmed on the turn path (stress bench, 2026-11-03).
//!
//! `WORKER_HISTORY_CACHE=off` disables it.

use std::sync::Arc;

use crate::core::RuntimeMessage;
use uuid::Uuid;

/// Upper bound on cached history, by wire size.
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// The latest messages of a session's history as of `source_sequence`: all
/// of it after a full read, or the latest window after a windowed one.
pub(crate) struct CachedHistory {
    pub(crate) messages: Vec<RuntimeMessage>,
    pub(crate) source_sequence: i64,
    /// Messages in the session's whole history as of `source_sequence`;
    /// `messages` holds the latest of them.
    pub(crate) total_count: usize,
    /// Wire size of `messages`, the cache weight.
    bytes: u64,
}

impl CachedHistory {
    /// Whether these messages, extended by `appended` more, are enough for a
    /// read of the latest `limit` messages (`None`: the whole history).
    pub(crate) fn covers(&self, appended: usize, limit: Option<usize>) -> bool {
        let total = self.total_count + appended;
        let wanted = limit.map_or(total, |limit| limit.min(total));
        self.messages.len() + appended >= wanted
    }
}

/// Shared by every clone of one [`super::GrpcClient`].
pub(crate) struct HistoryCache {
    sessions: Option<moka::sync::Cache<Uuid, Arc<CachedHistory>>>,
}

impl HistoryCache {
    pub(crate) fn from_env() -> Arc<Self> {
        let off = std::env::var("WORKER_HISTORY_CACHE")
            .is_ok_and(|v| v.eq_ignore_ascii_case("off") || v == "0" || v == "false");
        Arc::new(if off {
            Self::disabled()
        } else {
            Self::with_capacity(MAX_CACHE_BYTES)
        })
    }

    pub(crate) fn disabled() -> Self {
        Self { sessions: None }
    }

    pub(crate) fn with_capacity(max_bytes: u64) -> Self {
        let sessions = moka::sync::Cache::builder()
            .max_capacity(max_bytes)
            .weigher(|_, entry: &Arc<CachedHistory>| u32::try_from(entry.bytes).unwrap_or(u32::MAX))
            .build();
        Self {
            sessions: Some(sessions),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.sessions.is_some()
    }

    pub(crate) fn get(&self, session_id: Uuid) -> Option<Arc<CachedHistory>> {
        self.sessions.as_ref()?.get(&session_id)
    }

    /// Store `previous` (if any) extended by `appended`, read up to
    /// `source_sequence`, and return the messages held. Without `previous`,
    /// `total_count` is the session's message count the read reported;
    /// with it, the count grows by what was appended.
    pub(crate) fn extend(
        &self,
        session_id: Uuid,
        previous: Option<&CachedHistory>,
        appended: Vec<RuntimeMessage>,
        appended_bytes: u64,
        source_sequence: i64,
        total_count: usize,
    ) -> Vec<RuntimeMessage> {
        let (mut messages, bytes, total_count) = match previous {
            Some(previous) => (
                previous.messages.clone(),
                previous.bytes,
                previous.total_count + appended.len(),
            ),
            None => (Vec::new(), 0, total_count),
        };
        messages.extend(appended);
        if let Some(sessions) = &self.sessions {
            // Two loads of one session can race; never step a cached history
            // back to an older sequence.
            let newer = sessions
                .get(&session_id)
                .is_none_or(|cached| cached.source_sequence <= source_sequence);
            if newer {
                sessions.insert(
                    session_id,
                    Arc::new(CachedHistory {
                        messages: messages.clone(),
                        source_sequence,
                        total_count,
                        bytes: bytes + appended_bytes,
                    }),
                );
            }
        }
        messages
    }

    pub(crate) fn forget(&self, session_id: Uuid) {
        if let Some(sessions) = &self.sessions {
            sessions.invalidate(&session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
        messages.iter().map(|m| m.content_to_llm_string()).collect()
    }

    #[test]
    fn extend_appends_to_the_cached_history() {
        let cache = HistoryCache::with_capacity(MAX_CACHE_BYTES);
        let session = Uuid::now_v7();
        cache.extend(session, None, vec![RuntimeMessage::user("a")], 10, 1, 1);
        let cached = cache.get(session).unwrap();
        let all = cache.extend(
            session,
            Some(&cached),
            vec![RuntimeMessage::assistant("b")],
            10,
            2,
            0,
        );
        assert_eq!(texts(&all), ["a", "b"]);
        let cached = cache.get(session).unwrap();
        assert_eq!(cached.source_sequence, 2);
        assert_eq!(cached.total_count, 2);
        assert_eq!(cached.bytes, 20);
    }

    /// A windowed read caches the latest messages and the session's count:
    /// it serves later windows up to what it holds, never a full read.
    #[test]
    fn a_window_covers_reads_no_wider_than_itself() {
        let cache = HistoryCache::with_capacity(MAX_CACHE_BYTES);
        let session = Uuid::now_v7();
        // The latest 2 of 10 messages.
        cache.extend(
            session,
            None,
            vec![RuntimeMessage::user("i"), RuntimeMessage::assistant("j")],
            2,
            20,
            10,
        );
        let cached = cache.get(session).unwrap();
        assert_eq!(cached.total_count, 10);
        assert!(cached.covers(0, Some(2)));
        assert!(
            cached.covers(1, Some(3)),
            "one new message widens it by one"
        );
        assert!(!cached.covers(0, Some(3)));
        assert!(
            !cached.covers(5, None),
            "a full read needs the whole history"
        );

        let held = cache.extend(
            session,
            Some(&cached),
            vec![RuntimeMessage::user("k")],
            1,
            22,
            0,
        );
        assert_eq!(texts(&held), ["i", "j", "k"]);
        assert_eq!(cache.get(session).unwrap().total_count, 11);
    }

    #[test]
    fn a_whole_history_covers_every_read() {
        let cache = HistoryCache::with_capacity(MAX_CACHE_BYTES);
        let session = Uuid::now_v7();
        cache.extend(session, None, vec![RuntimeMessage::user("a")], 1, 1, 1);
        let cached = cache.get(session).unwrap();
        assert!(cached.covers(0, None));
        assert!(cached.covers(3, Some(50)));
    }

    /// A slower load that read less must not replace a newer cached history.
    #[test]
    fn extend_never_steps_back_to_an_older_sequence() {
        let cache = HistoryCache::with_capacity(MAX_CACHE_BYTES);
        let session = Uuid::now_v7();
        cache.extend(session, None, vec![RuntimeMessage::user("a")], 1, 5, 1);
        let returned = cache.extend(session, None, vec![RuntimeMessage::user("old")], 1, 3, 1);
        assert_eq!(
            texts(&returned),
            ["old"],
            "the caller still gets what it read"
        );
        assert_eq!(cache.get(session).unwrap().source_sequence, 5);
    }

    #[test]
    fn disabled_cache_stores_nothing() {
        let cache = HistoryCache::disabled();
        let session = Uuid::now_v7();
        assert!(!cache.enabled());
        let all = cache.extend(session, None, vec![RuntimeMessage::user("a")], 1, 1, 1);
        assert_eq!(texts(&all), ["a"]);
        assert!(cache.get(session).is_none());
    }

    #[test]
    fn forget_drops_the_entry() {
        let cache = HistoryCache::with_capacity(MAX_CACHE_BYTES);
        let session = Uuid::now_v7();
        cache.extend(session, None, vec![RuntimeMessage::user("a")], 1, 1, 1);
        cache.forget(session);
        assert!(cache.get(session).is_none());
    }
}
