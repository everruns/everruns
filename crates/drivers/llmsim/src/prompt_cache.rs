//! Simulated provider prompt caching and prefill time.
//!
//! Decisions:
//! - Caching follows OpenAI's automatic prefix cache: a request reuses the
//!   longest message prefix it shares with the session's previous request, if
//!   that request used the same model and is younger than the 5 minute TTL.
//!   Prompts under 1,024 tokens are never cached, and hits count in 128-token
//!   steps. So a long session reads most of its input from cache, and misses
//!   after compaction rewrites history, after a model switch, or after an idle
//!   gap, the way real long sessions do.
//! - The cache is keyed by the `session_id` call metadata. Calls without one
//!   (side calls, tests) are never cached.
//! - Prefill: uncached prompt tokens cost the latency profile's per-token
//!   prefill rate before the first token, and cached tokens a tenth of it. Turn
//!   900 of a long session therefore starts later than turn 1, and a cache
//!   miss after compaction or an idle gap shows up as a slower first token.
//!   Scenarios always use llmsim's default rate (about 20k tokens/s) because
//!   they model hosted providers even on the instant `llmsim-default` model.
//! - Token counts are the driver's usual estimate of about 4 characters per
//!   token.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;

use everruns_contracts::driver_registry::Message;
use tokio::time::Instant;

/// Time a cached prefix stays warm after its last use.
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// Shortest prompt a provider caches.
pub(crate) const MIN_CACHED_TOKENS: u32 = 1024;
/// Cache hits are counted in steps of this many tokens.
const CACHE_STEP: u32 = 128;
/// Sessions tracked at once; older entries are dropped past this.
const MAX_SESSIONS: usize = 100_000;

/// Prompt tokens of one call, split the way providers bill them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PromptUsage {
    /// Tokens processed from scratch.
    pub uncached: u32,
    /// Tokens read from the provider's prompt cache.
    pub cached: u32,
}

impl PromptUsage {
    /// Time spent reading the prompt before the first token at `us_per_token`.
    pub(crate) fn prefill(&self, us_per_token: u64) -> Duration {
        let micros =
            u64::from(self.uncached) * us_per_token + u64::from(self.cached) * us_per_token / 10;
        Duration::from_micros(micros)
    }

    /// The prompt tokens field of completion metadata (uncached only).
    pub(crate) fn prompt_tokens(&self) -> u32 {
        self.uncached
    }

    /// The cache read tokens field of completion metadata.
    pub(crate) fn cache_read_tokens(&self) -> Option<u32> {
        (self.cached > 0).then_some(self.cached)
    }
}

struct Entry {
    model: String,
    /// Cumulative (hash, tokens) after each message of the last request.
    prefix: Vec<(u64, u32)>,
    at: Instant,
}

/// Per-session record of the last request, shared by clones of one driver.
#[derive(Default)]
pub(crate) struct PromptCache {
    sessions: Mutex<HashMap<String, Entry>>,
}

/// About 4 characters per token, at least one per message.
pub(crate) fn estimate_tokens(text: &str) -> u32 {
    (text.len() / 4).max(1) as u32
}

fn prefix_of(messages: &[Message]) -> Vec<(u64, u32)> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut tokens = 0u32;
    messages
        .iter()
        .map(|message| {
            // The whole message, tool calls and reasoning included, decides a
            // cache hit; `Message` has no serde form, so hash its debug form.
            format!("{message:?}").hash(&mut hasher);
            tokens = tokens.saturating_add(estimate_tokens(&message.content_as_text()));
            (hasher.finish(), tokens)
        })
        .collect()
}

impl PromptCache {
    /// Prompt usage for this call, and record it as the session's latest.
    pub(crate) fn usage(
        &self,
        session: Option<&str>,
        model: &str,
        messages: &[Message],
    ) -> PromptUsage {
        let prefix = prefix_of(messages);
        let total = prefix.last().map_or(0, |(_, tokens)| *tokens);
        let Some(session) = session else {
            return PromptUsage {
                uncached: total,
                cached: 0,
            };
        };
        let now = Instant::now();
        let Ok(mut sessions) = self.sessions.lock() else {
            return PromptUsage {
                uncached: total,
                cached: 0,
            };
        };

        let shared = sessions
            .get(session)
            .filter(|entry| entry.model == model && now.duration_since(entry.at) < CACHE_TTL)
            .map_or(0, |entry| {
                entry
                    .prefix
                    .iter()
                    .zip(&prefix)
                    .take_while(|(old, new)| old.0 == new.0)
                    .last()
                    .map_or(0, |(_, new)| new.1)
            });
        let cached = if shared >= MIN_CACHED_TOKENS {
            shared / CACHE_STEP * CACHE_STEP
        } else {
            0
        };

        if sessions.len() >= MAX_SESSIONS && !sessions.contains_key(session) {
            sessions.retain(|_, entry| now.duration_since(entry.at) < CACHE_TTL);
            if sessions.len() >= MAX_SESSIONS {
                sessions.clear();
            }
        }
        sessions.insert(
            session.to_string(),
            Entry {
                model: model.to_string(),
                prefix,
                at: now,
            },
        );
        PromptUsage {
            uncached: total - cached,
            cached,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::driver_registry::MessageRole;

    fn big(text: &str) -> Message {
        // 2,000 tokens at about 4 characters each.
        Message::text(MessageRole::User, format!("{text} {}", "x".repeat(8000)))
    }

    #[test]
    fn small_prompts_and_unknown_sessions_are_never_cached() {
        let cache = PromptCache::default();
        let short = [Message::text(MessageRole::User, "hello")];
        cache.usage(Some("s"), "m", &short);
        assert_eq!(cache.usage(Some("s"), "m", &short).cached, 0);

        let long = [big("a")];
        cache.usage(None, "m", &long);
        let usage = cache.usage(None, "m", &long);
        assert_eq!(usage.cached, 0);
        assert!(usage.uncached >= 2000);
    }

    #[tokio::test(start_paused = true)]
    async fn a_growing_session_reads_its_prefix_from_cache() {
        let cache = PromptCache::default();
        let mut messages = vec![big("system"), big("first")];
        let first = cache.usage(Some("s"), "m", &messages);
        assert_eq!(first.cached, 0);

        messages.push(Message::text(MessageRole::Assistant, "answer"));
        messages.push(Message::text(MessageRole::User, "next"));
        let second = cache.usage(Some("s"), "m", &messages);
        assert_eq!(second.cached, first.uncached / CACHE_STEP * CACHE_STEP);
        assert_eq!(second.cached + second.uncached, first.uncached + 2);
        assert!(second.prefill(50) < first.prefill(50));
    }

    #[tokio::test(start_paused = true)]
    async fn rewrites_model_switches_and_idle_gaps_miss() {
        let cache = PromptCache::default();
        let messages = vec![big("system"), big("history")];
        cache.usage(Some("s"), "m", &messages);

        // Compaction rewrites the first message: nothing is shared.
        let compacted = vec![big("summary"), big("history")];
        assert_eq!(cache.usage(Some("s"), "m", &compacted).cached, 0);

        // A different model has its own cache.
        assert_eq!(cache.usage(Some("s"), "other", &compacted).cached, 0);
        assert!(cache.usage(Some("s"), "other", &compacted).cached > 0);

        // Past the TTL the prefix is cold again.
        tokio::time::advance(CACHE_TTL + Duration::from_secs(1)).await;
        assert_eq!(cache.usage(Some("s"), "other", &compacted).cached, 0);
    }

    #[test]
    fn cached_tokens_prefill_ten_times_faster() {
        let cold = PromptUsage {
            uncached: 100_000,
            cached: 0,
        };
        let warm = PromptUsage {
            uncached: 0,
            cached: 100_000,
        };
        assert_eq!(cold.prefill(50), Duration::from_secs(5));
        assert_eq!(warm.prefill(50), Duration::from_millis(500));
    }
}
