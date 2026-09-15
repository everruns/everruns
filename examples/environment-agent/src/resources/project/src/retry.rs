//! Decide whether a failed request should be tried again.

pub fn should_retry(attempt: u32, status: u16) -> bool {
    if attempt >= 3 {
        return false;
    }
    matches!(status, 429 | 500..=599)
}

// TODO: honor the Retry-After header when the server sends one.
pub fn backoff_millis(attempt: u32) -> u64 {
    100 * 2_u64.pow(attempt)
}
