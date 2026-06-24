//! Shared low-level security primitives.

/// Compare two byte slices in constant time with respect to their contents.
///
/// Secret/credential comparisons (passwords, hashed API keys and client
/// secrets, PKCE challenges, HMAC signatures) must never use a short-circuiting
/// `==`/`!=`: those return as soon as the first differing byte is found, leaking
/// a timing side-channel an attacker can use to recover the secret byte by byte.
///
/// This returns `false` immediately when the lengths differ (length is not
/// treated as secret) and otherwise inspects every byte without short-circuiting
/// so the running time does not depend on the position of the first mismatch.
///
/// This is the single canonical implementation for the server crate — do not
/// re-roll per-module copies.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn matches_equal_slices() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn rejects_differing_contents() {
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn rejects_differing_lengths() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
