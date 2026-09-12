//! Minimal HTTP client stub for the sample repository.

/// Retry policy applied to every request.
pub struct Retry {
    pub attempts: u32,
    pub backoff_ms: u64,
}

impl Default for Retry {
    fn default() -> Self {
        Self { attempts: 3, backoff_ms: 250 }
    }
}
