//! Deterministic credential-format detection for text a person typed.
//!
//! The `secret` question kind (EVE-1058) gives a credential a safe path: the
//! value goes to the encrypted session-secret store and the model is handed a
//! name to resolve. This module closes the unsafe path (EVE-1059). A plain
//! choice question with `allow_other: true` renders a free-text box, and a
//! person can paste an API key into it — and *that* text becomes a tool result,
//! persisted in `events` and replayed into model context every turn. There is
//! no un-persisting an event, so the check has to run before the write rather
//! than redact after it. This is TM-AGENT-016's mechanism arriving through a
//! new door.
//!
//! Deterministic by design. `knowledge/security/secret-leak-guardrails.md`
//! splits the families: deterministic checks catch known *formats*, model-backed
//! judges catch semantic intent. This runs synchronously on a person's keystroke
//! path, so a utility-LLM call is the wrong cost, and a known-format match is
//! the right layer.
//!
//! This is the one place the format list lives. Extend [`PREFIX_RULES`] rather
//! than adding a second list somewhere else. The patterns mirror
//! `scripts/scan_actions_log_secrets.py`, which scans Actions logs for the same
//! formats.

use regex::Regex;
use std::sync::LazyLock;

/// A credential format, named so a rejection can say what it looked like
/// without ever echoing the value.
pub struct CredentialFormat {
    pub label: &'static str,
    pattern: Regex,
}

/// Formats that are a credential wherever they appear.
///
/// Prefix-anchored on purpose. The issue's tuning note is the reason: a person
/// answering with a legitimate long opaque identifier — an order number, a
/// trace id, a content hash — must not be blocked, and a generic entropy or
/// run-length heuristic cannot tell those from a key. A missed credential is
/// recoverable (the person can still use the secret flow); a false positive on
/// an ordinary answer is a dead end in the middle of a conversation.
static PREFIX_RULES: LazyLock<Vec<CredentialFormat>> = LazyLock::new(|| {
    [
        // `sk-ant-` is tried before the general `sk-` so an Anthropic key is
        // named as one.
        ("an Anthropic API key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
        ("an OpenAI API key", r"sk-[A-Za-z0-9_\-]{20,}"),
        (
            "a GitHub token",
            r"gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,}",
        ),
        (
            "a Doppler token",
            r"dp\.(st|pt|sa|ct|scim)\.[A-Za-z0-9_.\-]{20,}",
        ),
        ("a Slack token", r"xox[baprs]-[A-Za-z0-9\-]{10,}"),
        ("an AWS access key id", r"AKIA[0-9A-Z]{16}"),
        ("a GitLab token", r"glpat-[A-Za-z0-9_\-]{20,}"),
        ("a TypeSafe API key", r"apikey_[A-Za-z0-9]{24,}"),
        ("an Everruns token", r"evr_(pat|a2a|app)_[A-Za-z0-9]{16,}"),
        // A PEM block is a private key wherever it appears, and the header
        // alone is enough — nobody types this by accident.
        (
            "a private key block",
            r"-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----",
        ),
    ]
    .into_iter()
    .map(|(label, pattern)| CredentialFormat {
        label,
        // Static patterns, compiled once. A bad one is a bug, not input.
        pattern: Regex::new(pattern).expect("credential pattern compiles"),
    })
    .collect()
});

/// Whether `byte` can be part of the token a credential lives in.
///
/// A real credential never starts mid-word. Without this, `sk-` matched inside
/// the ordinary word "ask-", and a link to
/// `EVE-1053/ask-user-capability-contract-schema-and-knowledge-spec` reported
/// as an OpenAI key — the same false positive that turned the hourly Actions
/// log sweep red on every branch. Requiring a token boundary costs no
/// detection, because a credential is never preceded by a word character.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// The format `text` looks like, if it looks like a credential at all.
///
/// Returns the label only. The matched value is deliberately never returned or
/// logged: a check that exists to keep a credential out of the event log must
/// not put it in an error message instead.
pub fn credential_format(text: &str) -> Option<&'static str> {
    let bytes = text.as_bytes();
    for rule in PREFIX_RULES.iter() {
        for found in rule.pattern.find_iter(text) {
            let starts_a_token = found.start() == 0 || !is_token_byte(bytes[found.start() - 1]);
            if starts_a_token {
                return Some(rule.label);
            }
        }
    }
    None
}

/// Why a credential-shaped answer was refused, and what to do instead.
///
/// Phrased so a false positive is a small annoyance rather than a dead end: it
/// names what the text resembled and points at the flow that handles it safely.
pub fn credential_rejection(question_id: &str, label: &str) -> String {
    format!(
        "question {question_id:?} free text looks like {label}. A free-text answer is stored in the \
         session's event log and replayed into model context every turn, so it is not a safe place \
         for a credential. Ask for it with a `secret` question instead, which stores the value \
         encrypted and hands the model only a reference. If this is not a credential, rephrase the \
         answer so it does not begin with a recognised key prefix."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_credential_formats_are_caught() {
        for (text, expected) in [
            (
                "sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345",
                "an Anthropic API key",
            ),
            ("sk-abcdefghijklmnopqrstuvwxyz012345", "an OpenAI API key"),
            (
                "ghp_abcdefghijklmnopqrstuvwxyz0123456789AB",
                "a GitHub token",
            ),
            ("AKIAIOSFODNN7EXAMPLE", "an AWS access key id"),
            ("xoxb-123456789012-abcdefghijkl", "a Slack token"),
            ("glpat-abcdefghijklmnopqrstuvwx", "a GitLab token"),
            ("dp.st.prd.abcdefghijklmnopqrstuvwxyz", "a Doppler token"),
            ("apikey_abcdefghijklmnopqrstuvwxyz01", "a TypeSafe API key"),
            ("evr_pat_abcdefghijklmnopqr", "an Everruns token"),
            ("-----BEGIN RSA PRIVATE KEY-----", "a private key block"),
            ("-----BEGIN PRIVATE KEY-----", "a private key block"),
        ] {
            assert_eq!(credential_format(text), Some(expected), "missed {text:?}");
        }
    }

    #[test]
    fn a_credential_is_caught_inside_a_sentence() {
        assert_eq!(
            credential_format("use sk-abcdefghijklmnopqrstuvwxyz012345 for this"),
            Some("an OpenAI API key")
        );
        assert_eq!(
            credential_format("token=ghp_abcdefghijklmnopqrstuvwxyz0123456789AB"),
            Some("a GitHub token")
        );
    }

    /// The regression that made this rule exist: `sk-` inside "ask-".
    #[test]
    fn a_prefix_inside_a_word_is_not_a_credential() {
        for text in [
            "EVE-1053/ask-user-capability-contract-schema-and-knowledge-spec",
            "please ask-user-about-the-deployment-window-before-proceeding",
            "task-abcdefghijklmnopqrstuvwxyz012345",
        ] {
            assert_eq!(credential_format(text), None, "false positive on {text:?}");
        }
    }

    /// The tuning bar: an ordinary answer, including a long opaque one, passes.
    #[test]
    fn ordinary_answers_including_long_ones_pass_through() {
        for text in [
            "Production",
            "Deploy to staging first, then production after the smoke test passes.",
            // A long opaque identifier a person might legitimately paste.
            "order 9f8e7d6c5b4a39281706fedcba9876543210fedcba9876543210fedcba987654",
            "trace_id 473885e823acd1a26387b8446ede28d1",
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "https://example.com/a/very/long/path/that/goes/on/for/quite/a/while",
            // Short enough not to be a key, even with the prefix shape.
            "sk-short",
        ] {
            assert_eq!(credential_format(text), None, "false positive on {text:?}");
        }
    }

    /// The message has to be actionable, and must never carry the value.
    #[test]
    fn the_rejection_names_the_safe_path_and_not_the_secret() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz012345";
        let label = credential_format(secret).unwrap();
        let message = credential_rejection("target", label);

        assert!(message.contains("secret"), "no path forward: {message}");
        assert!(!message.contains(secret), "the message leaked the value");
    }
}
