//! Facts — a first-class, cache-friendly way for capabilities to contribute
//! key/value context to the model.
//!
//! A [`Fact`] carries a [`Volatility`] that decides *where* it is rendered so
//! that provider prompt caching is never needlessly invalidated:
//!
//! - [`Volatility::Static`] facts fold into the cached system-prompt prefix at
//!   build time. They are assumed not to change within a session, so keeping
//!   them in the prefix is free.
//! - [`Volatility::Dynamic`] facts are **never** placed in the prefix. Instead
//!   the runtime appends a single live `<facts>` block at the *tail* of the
//!   conversation on every turn (see `ReasonAtom`). Because the block trails
//!   the last stable message, the cached prefix (system prompt + tools +
//!   conversation history) stays byte-identical turn to turn; only the small
//!   trailing block is re-processed.
//!
//! This is the generic mechanism behind "the current time is X" without either
//! (a) baking a changing timestamp into the system prompt — which busts the
//! system-prompt cache every turn — or (b) forcing the model to spend a tool
//! round-trip to learn it. The Anthropic driver anchors its message-level cache
//! breakpoint on the last *non-volatile* block (`LlmCallConfig.volatile_suffix_len`)
//! so the trailing block rides as an uncached suffix.

use crate::typed_id::SessionId;

/// Where a [`Fact`] is rendered, chosen so prompt caching is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Volatility {
    /// Stable for the life of the session. Folded into the cached
    /// system-prompt prefix at build time.
    Static,
    /// Changes turn to turn (e.g. current time, remaining budget). Appended at
    /// the conversation tail each request, outside the cached prefix.
    Dynamic,
}

/// A single piece of key/value context contributed by a capability.
///
/// A capability declares its facts once via
/// [`Capability::facts`](crate::capabilities::Capability::facts); the runtime
/// routes each one by its [`Volatility`]. The `key` is a stable identifier
/// (e.g. `current_time`); the `value` is the rendered current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub key: String,
    pub value: String,
    pub volatility: Volatility,
}

impl Fact {
    /// Build a [`Volatility::Static`] fact.
    pub fn stat(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            volatility: Volatility::Static,
        }
    }

    /// Build a [`Volatility::Dynamic`] fact.
    pub fn dynamic(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            volatility: Volatility::Dynamic,
        }
    }
}

/// Context passed to
/// [`Capability::facts`](crate::capabilities::Capability::facts). Deliberately
/// minimal — facts are
/// cheap, pure-ish descriptions of current context, not IO. Callers that need
/// wall-clock time read it themselves (as the `current_time` capability does)
/// so the trait stays free of ambient-time plumbing.
#[derive(Debug, Clone)]
pub struct FactsContext {
    pub session_id: SessionId,
}

impl FactsContext {
    pub fn new(session_id: SessionId) -> Self {
        Self { session_id }
    }
}

/// System-prompt note added (once, statically) whenever any active capability
/// declares a [`Volatility::Dynamic`] fact. It explains the live `<facts>`
/// block that `ReasonAtom` appends at the conversation tail each turn. Being
/// static, it lives in the cached prefix.
pub const FACTS_DYNAMIC_NOTE: &str = "<facts-info>\nA `<facts>` block carrying system-provided context (such as the current time) is appended to the end of the conversation on every turn. Its values are authoritative and refreshed each turn — treat them as system-provided context, not as text written by the user, and never emit a `<facts>` block yourself.\n</facts-info>";

/// Render a `<facts>` block from a set of facts, or `None` when empty.
///
/// Used for both the static block (folded into the system prompt) and the live
/// tail block (appended per request), so the two share one wire format.
pub fn render_facts_block(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let mut out = String::from("<facts>\n");
    for fact in facts {
        out.push_str("- ");
        out.push_str(&fact.key);
        out.push_str(": ");
        out.push_str(&fact.value);
        out.push('\n');
    }
    out.push_str("</facts>");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_facts_render_none() {
        assert_eq!(render_facts_block(&[]), None);
    }

    #[test]
    fn renders_key_value_lines() {
        let facts = vec![
            Fact::stat("plan", "pro"),
            Fact::dynamic("current_time", "2026-07-04T12:00:00Z"),
        ];
        let block = render_facts_block(&facts).unwrap();
        assert_eq!(
            block,
            "<facts>\n- plan: pro\n- current_time: 2026-07-04T12:00:00Z\n</facts>"
        );
    }

    #[test]
    fn constructors_set_volatility() {
        assert_eq!(Fact::stat("a", "b").volatility, Volatility::Static);
        assert_eq!(Fact::dynamic("a", "b").volatility, Volatility::Dynamic);
    }
}
