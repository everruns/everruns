//! Opaque reasoning replay state sized like a real provider's.
//!
//! Decision: OpenAI returns reasoning as `encrypted_content` of several KB per
//! reasoning item, and the runtime stores it and sends it back every turn. A
//! fixed 23-byte stand-in made simulated histories grow far slower than real
//! ones with reasoning on. The payload is now about 6 bytes per reasoning
//! token, at least 1 KB and at most 64 KB, and keeps the
//! `llmsim-encrypted-opaque` prefix that tests look for.

const PREFIX: &str = "llmsim-encrypted-opaque:";
const BYTES_PER_TOKEN: usize = 6;
const MIN_BYTES: usize = 1024;
const MAX_BYTES: usize = 64 * 1024;

/// An opaque payload standing in for `reasoning_tokens` of reasoning.
pub(crate) fn encrypted_payload(reasoning_tokens: usize) -> String {
    let len = (reasoning_tokens * BYTES_PER_TOKEN).clamp(MIN_BYTES, MAX_BYTES);
    // Base64-looking filler, deterministic so replays compare equal.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut payload = String::with_capacity(len);
    payload.push_str(PREFIX);
    let mut i = reasoning_tokens;
    while payload.len() < len {
        i = i.wrapping_mul(31).wrapping_add(7);
        payload.push(ALPHABET[i % ALPHABET.len()] as char);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_grows_with_reasoning_within_bounds() {
        assert_eq!(encrypted_payload(8).len(), MIN_BYTES);
        assert_eq!(encrypted_payload(1_000).len(), 6_000);
        assert_eq!(encrypted_payload(1_000_000).len(), MAX_BYTES);
        assert!(encrypted_payload(500).starts_with("llmsim-encrypted-opaque"));
        assert_eq!(encrypted_payload(500), encrypted_payload(500));
    }
}
