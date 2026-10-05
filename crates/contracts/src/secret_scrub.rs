//! Value-pattern credential scrubbing shared by every path that publishes
//! free-form strings: eval/ATIF exports (server) and the executed arguments on
//! `tool.completed` (engine). One pattern list, so a new credential shape is
//! covered everywhere at once.

use std::sync::LazyLock;

use regex::Regex;

/// Placeholder that replaces a scrubbed credential.
pub const REDACTED: &str = "[REDACTED]";

/// High-signal credential patterns. Deliberately conservative to avoid
/// mangling legitimate content.
#[expect(
    clippy::expect_used,
    reason = "Built-in constant regexes must compile; invalid syntax is a programming error"
)]
static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // OpenAI / Anthropic style keys: sk-..., sk-ant-...
        r"\bsk-[A-Za-z0-9_-]{16,}\b",
        // AWS access key id
        r"\bAKIA[0-9A-Z]{16}\b",
        // GitHub tokens
        r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
        // Bearer tokens
        r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{16,}",
        // key/secret/password/token assignments: api_key=..., "secret": "..."
        r#"(?i)\b(api[_-]?key|secret|password|passwd|token|access[_-]?key)\b\s*["']?\s*[:=]\s*["']?[A-Za-z0-9._~+/=-]{6,}"#,
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid secret regex"))
    .collect()
});

/// URL userinfo (`scheme://user:pass@host`, `scheme://token@host`). The scheme
/// is kept so the value still reads as a URL; the whole userinfo is replaced
/// because a bare user part is often the token itself.
#[expect(
    clippy::expect_used,
    reason = "Built-in constant regexes must compile; invalid syntax is a programming error"
)]
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://)[^/\s@?#]+@").expect("valid userinfo regex")
});

/// Scrub credential-looking substrings from a single string.
pub fn scrub_secrets(input: &str) -> String {
    let mut out = URL_USERINFO
        .replace_all(input, format!("${{1}}{REDACTED}@"))
        .into_owned();
    for re in SECRET_PATTERNS.iter() {
        out = re.replace_all(&out, REDACTED).into_owned();
    }
    out
}

/// Scrub every string leaf of `value` in place, recursing into arrays and
/// objects. Keys are left alone; key-based redaction is the caller's policy.
pub fn scrub_secrets_in_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => *s = scrub_secrets(s),
        serde_json::Value::Array(items) => items.iter_mut().for_each(scrub_secrets_in_value),
        serde_json::Value::Object(map) => map.values_mut().for_each(scrub_secrets_in_value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scrubs_provider_keys() {
        let scrubbed = scrub_secrets(
            "sk-abcdef0123456789ABCDEF AKIAABCDEFGHIJKLMNOP ghp_abcdefghijklmnopqrstuvwxyz",
        );
        assert_eq!(scrubbed, "[REDACTED] [REDACTED] [REDACTED]");
    }

    #[test]
    fn scrubs_bearer_header_in_command() {
        let scrubbed =
            scrub_secrets("curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig' x");
        assert!(!scrubbed.contains("eyJhbGci"), "{scrubbed}");
        assert!(scrubbed.contains(REDACTED));
    }

    #[test]
    fn scrubs_url_userinfo() {
        assert_eq!(
            scrub_secrets("git clone https://user:hunter2@github.com/o/r.git"),
            "git clone https://[REDACTED]@github.com/o/r.git"
        );
        assert_eq!(
            scrub_secrets("postgres://app:pw@db:5432/x and https://tok123@example.com"),
            "postgres://[REDACTED]@db:5432/x and https://[REDACTED]@example.com"
        );
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "https://example.com/path?q=a@b",
            "mail me at dev@example.com",
            "ls -la /tmp",
            "max_tokens: 5",
        ] {
            assert_eq!(scrub_secrets(text), text);
        }
    }

    #[test]
    fn scrubs_nested_string_leaves() {
        let mut value = json!({
            "cmd": ["sh", "-c", "export K=sk-abcdef0123456789ABCDEF"],
            "nested": { "list": [{ "url": "https://u:p@h/x" }] },
            "n": 3,
        });
        scrub_secrets_in_value(&mut value);
        assert_eq!(
            value,
            json!({
                "cmd": ["sh", "-c", "export K=[REDACTED]"],
                "nested": { "list": [{ "url": "https://[REDACTED]@h/x" }] },
                "n": 3,
            })
        );
    }
}
