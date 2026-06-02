// Per-org caps for concurrent sessions and active turns (EVE-508).
//
// Defaults are generous — designed to bound abuse without throttling
// legitimate long-horizon agentic workloads.
// Operators can tighten via env without redeploying code.

const DEFAULT_MAX_CONCURRENT_SESSIONS: usize = 10_000;
const DEFAULT_MAX_ACTIVE_TURNS: usize = 1_000;

/// Per-org concurrency caps for sessions and turns.
#[derive(Clone, Copy, Debug)]
pub struct OrgCaps {
    /// Max non-finished sessions per org simultaneously.
    pub max_concurrent_sessions: usize,
    /// Max sessions with an actively executing turn per org simultaneously.
    pub max_active_turns: usize,
}

impl Default for OrgCaps {
    fn default() -> Self {
        Self::from_env()
    }
}

impl OrgCaps {
    pub fn from_env() -> Self {
        Self {
            max_concurrent_sessions: read_usize_env(
                "ORG_MAX_CONCURRENT_SESSIONS",
                DEFAULT_MAX_CONCURRENT_SESSIONS,
            ),
            max_active_turns: read_usize_env("ORG_MAX_ACTIVE_TURNS", DEFAULT_MAX_ACTIVE_TURNS),
        }
    }
}

fn read_usize_env(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}
