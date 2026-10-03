use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InfinityContextConfig {
    /// Maximum prompt budget reserved for message history.
    #[serde(default = "default_context_budget_tokens")]
    pub(super) context_budget_tokens: usize,

    /// Minimum number of recent messages to keep even when the budget is tight.
    #[serde(default = "default_min_recent_messages")]
    pub(super) min_recent_messages: usize,

    /// Optional hard cap on recent messages kept in the live prompt.
    ///
    /// Useful for public support chats where the prompt must stay small even
    /// when the token-budget estimate would allow more messages.
    #[serde(default)]
    pub(super) max_recent_messages: Option<usize>,

    /// Optional leading messages kept as an anchor (the original task / goal),
    /// regardless of token budget. Defaults to 0 so untrusted first messages
    /// cannot bypass the configured token budget or recent-message cap. The
    /// anchor is additional to `max_recent_messages` when explicitly enabled.
    #[serde(default = "default_keep_first_messages")]
    pub(super) keep_first_messages: usize,

    /// Derived (not user-facing): set by capability collection when the
    /// `compaction` capability is also enabled. When true, infinity context
    /// stops doing token-budget eviction and lets compaction own reduction, so
    /// compaction's summary — not a bare "hidden" notice — covers old turns.
    #[serde(default)]
    pub(super) compaction_active: bool,

    /// Derived: the resolved provider preserves and reduces the complete wire
    /// transcript, so Infinity Context must not window or annotate it.
    #[serde(default)]
    pub(super) provider_managed_reduction: bool,
}

pub(super) fn default_context_budget_tokens() -> usize {
    100_000
}

pub(super) fn default_min_recent_messages() -> usize {
    10
}

pub(super) fn default_keep_first_messages() -> usize {
    0
}

impl Default for InfinityContextConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_context_budget_tokens(),
            min_recent_messages: default_min_recent_messages(),
            max_recent_messages: None,
            keep_first_messages: default_keep_first_messages(),
            compaction_active: false,
            provider_managed_reduction: false,
        }
    }
}

pub(super) const CANDIDATE_AVG_TOKENS_PER_MESSAGE: usize = 250;
pub(super) const CANDIDATE_OVERFETCH_FACTOR: usize = 4;
pub(super) const CANDIDATE_MAX_MESSAGES: usize = 2_000;
pub(super) const MAX_KEEP_FIRST_MESSAGES: usize = 16;
