//! Value-pattern credential scrubbing shared by every path that publishes
//! free-form strings: eval/ATIF exports (server) and the executed arguments on
//! `tool.completed` (engine). One pattern list, so a new credential shape is
//! covered everywhere at once.

use std::sync::LazyLock;

use regex::Regex;

/// Placeholder that replaces a scrubbed credential.
pub const REDACTED: &str = "[REDACTED]";

/// Key fragments (lowercased, `-`/`_` removed) that mark a JSON field as
/// carrying a credential. `contains` matching covers compound names like
/// `openai_api_key`, `client_secret`, or `x-api-key`.
pub const CREDENTIAL_KEY_FRAGMENTS: [&str; 10] = [
    "apikey",
    "accesskey",
    "secretkey",
    "privatekey",
    "secret",
    "password",
    "passwd",
    "credential",
    "authorization",
    "cookie",
];

/// Whether a JSON field named `key` holds a credential, whatever its value
/// looks like.
pub fn is_credential_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    // `token` only as a suffix: `access_token` / `refreshToken` are
    // credentials, while token *counts* (`max_tokens`, `input_tokens`) are not.
    normalized.ends_with("token")
        || CREDENTIAL_KEY_FRAGMENTS
            .iter()
            .any(|fragment| normalized.contains(fragment))
}

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

// Match whole field names, not cookie_count, X-Cookie, or prose mentioning cookies.
#[expect(
    clippy::expect_used,
    reason = "Built-in constant regexes must compile; invalid syntax is a programming error"
)]
static COOKIE_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(^|[^\w-])(set-cookie|cookie)((?:\\?["'])?)[ \t]*([:=])[ \t]*"#)
        .expect("valid cookie field regex")
});

fn scrub_cookies(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    for captures in COOKIE_FIELD.captures_iter(input) {
        let Some(field) = captures.get(0) else {
            continue;
        };
        if field.start() < copied {
            continue;
        }
        let Some(name) = captures.get(2) else {
            continue;
        };
        let mut start = field.end();
        let assignment = &captures[4] == "=";
        // A quote can enclose the value, the entire shell header, or an
        // escaped header inside serialized command arguments.
        let quote_at = |offset: usize| -> Option<(u8, usize)> {
            let tail = input.as_bytes().get(offset..)?;
            match tail {
                [q @ (b'\'' | b'"'), ..] => Some((*q, 0)),
                [b'\\', q @ (b'\'' | b'"'), ..] => Some((*q, 1)),
                _ => None,
            }
        };
        let delimiter = if let Some((quote, escapes)) = quote_at(start) {
            start += escapes + 1;
            Some((quote, escapes))
        } else if captures[3].is_empty() && name.start() > 0 {
            let before = &input.as_bytes()[..name.start()];
            let quote = before[before.len() - 1];
            matches!(quote, b'\'' | b'"').then(|| {
                let escapes = usize::from(before.len() > 1 && before[before.len() - 2] == b'\\');
                (quote, escapes)
            })
        } else {
            None
        };
        let bytes = input.as_bytes();
        let mut end = start;
        while end < bytes.len() {
            if matches!(bytes[end], b'\r' | b'\n') {
                break;
            }
            if let Some((quote, escapes)) = delimiter {
                if bytes[end] == quote {
                    let slashes = bytes[start..end]
                        .iter()
                        .rev()
                        .take_while(|&&b| b == b'\\')
                        .count();
                    if slashes == escapes {
                        end -= escapes;
                        break;
                    }
                }
            } else if assignment && (bytes[end].is_ascii_whitespace() || bytes[end] == b';') {
                break;
            }
            end += 1;
        }
        let value = &input[start..end];
        // Header prose is not a cookie. Actual headers contain name=value;
        // assignments can also carry opaque values of any length.
        if value.is_empty() || (!assignment && !value.contains('=') && value != REDACTED) {
            continue;
        }
        out.push_str(&input[copied..start]);
        out.push_str(REDACTED);
        copied = end;
    }
    out.push_str(&input[copied..]);
    out
}

/// Scrub credential-looking substrings from a single string.
pub fn scrub_secrets(input: &str) -> String {
    let cookies = scrub_cookies(input);
    let mut out = URL_USERINFO
        .replace_all(&cookies, format!("${{1}}{REDACTED}@"))
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
    fn credential_keys_match_compound_names_but_not_token_counts() {
        for key in [
            "api_key",
            "X-Api-Key",
            "client_secret",
            "Authorization",
            "Set-Cookie",
            "access_token",
            "refreshToken",
            "db_password",
            "aws_credentials",
        ] {
            assert!(is_credential_key(key), "{key}");
        }
        for key in ["max_tokens", "input_tokens", "url", "command", "author"] {
            assert!(!is_credential_key(key), "{key}");
        }
    }

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
    fn scrubs_cookie_headers_and_assignments() {
        for (input, expected) in [
            ("Cookie: sid=abc; csrf=def", "Cookie: [REDACTED]"),
            (
                "Set-Cookie: sid=abc; Path=/; HttpOnly\nContent-Type: text/plain",
                "Set-Cookie: [REDACTED]\nContent-Type: text/plain",
            ),
            (
                "curl -H 'Cookie: sid=abc; csrf=def' https://example.com",
                "curl -H 'Cookie: [REDACTED]' https://example.com",
            ),
            (
                r#"curl -H "cOoKiE: sid=abc; csrf=def" x"#,
                r#"curl -H "cOoKiE: [REDACTED]" x"#,
            ),
            (
                r#"export COOKIE="sid=abc; csrf=def"; echo ok"#,
                r#"export COOKIE="[REDACTED]"; echo ok"#,
            ),
            (
                r#"{"Cookie": "sid=abc; csrf=def", "safe": true}"#,
                r#"{"Cookie": "[REDACTED]", "safe": true}"#,
            ),
            (
                r#"curl -H 'Cookie: sid="abc def"; csrf=ghi' x"#,
                r#"curl -H 'Cookie: [REDACTED]' x"#,
            ),
            (
                r#"curl -H \"Cookie: sid=abc; csrf=def\" x"#,
                r#"curl -H \"Cookie: [REDACTED]\" x"#,
            ),
            (
                r#"{\"Cookie\": \"sid=abc; csrf=def\", \"safe\": true}"#,
                r#"{\"Cookie\": \"[REDACTED]\", \"safe\": true}"#,
            ),
            (
                r#"curl -H "Cookie: sid=\"abc def\"; csrf=ghi" x"#,
                r#"curl -H "Cookie: [REDACTED]" x"#,
            ),
            ("COOKIE=abc command", "COOKIE=[REDACTED] command"),
            ("Cookie: sid=短; csrf=令牌", "Cookie: [REDACTED]"),
            (r#"COOKIE='abc' command"#, r#"COOKIE='[REDACTED]' command"#),
        ] {
            assert_eq!(scrub_secrets(input), expected, "{input}");
            assert_eq!(scrub_secrets(expected), expected, "idempotence: {input}");
        }
    }

    #[test]
    fn cookie_words_and_unrelated_assignments_are_not_credentials() {
        for input in [
            "cookie recipe",
            "cookie: chocolate chip",
            "cookie_count=3",
            "mycookie=abc",
            "X-Cookie: example",
            "set-cookie-policy=strict",
        ] {
            assert_eq!(scrub_secrets(input), input);
        }
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
