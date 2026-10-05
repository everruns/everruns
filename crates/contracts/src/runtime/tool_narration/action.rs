//! Shared presentation for tool-owned action wording and explicitly selected details.

use super::{ToolNarrationPhase, is_uk, labeled_phrase, safe_arg_str, truncate, url_display};
use serde_json::Value;
use std::sync::LazyLock;

#[expect(
    clippy::unwrap_used,
    reason = "constant URL regex is validated by unit tests"
)]
static URL: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"(?i)https?://[^\s<>)\]\"']+"#).unwrap());

/// A bounded, single-line display value. URLs never echo credentials or query strings.
pub fn narration_detail(value: &str) -> String {
    let redacted = URL.replace_all(value, |captures: &regex::Captures<'_>| {
        url_display(&captures[0])
    });
    let words = redacted
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|ch| {
                    !ch.is_control()
                        && !matches!(*ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    truncate(&words, 80)
}

pub(super) fn resource_detail(value: &str) -> String {
    if url::Url::parse(value).is_ok_and(|url| url.host().is_some()) {
        narration_detail(value)
    } else {
        narration_detail(super::basename(value))
    }
}

/// Tools select their own wording and safe label fields; never inspect arbitrary arguments.
pub fn narrate_labeled_action(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
    english: (&str, &str, &str),
    ukrainian: (&str, &str, &str),
    detail_keys: &[&str],
) -> String {
    let verbs = if is_uk(locale) { ukrainian } else { english };
    let detail = detail_keys.iter().find_map(|key| {
        safe_arg_str(arguments, &[*key]).map(|value| {
            if key.ends_with("path") || *key == "filename" {
                resource_detail(value)
            } else {
                narration_detail(value)
            }
        })
    });
    labeled_phrase(verbs.0, verbs.1, verbs.2, detail, phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn details_are_bounded_single_line_and_strip_url_credentials() {
        assert_eq!(narration_detail("  Report\n\tready\r\n  "), "Report ready");
        assert_eq!(
            narration_detail("Visit https://user:password@example.com/report?token=secret#private"),
            "Visit example.com/report"
        );
        assert_eq!(
            narration_detail(&"é".repeat(100)),
            format!("{}...", "é".repeat(80))
        );
        assert_eq!(narration_detail("a\0b"), "ab");
        assert_eq!(
            narration_detail(
                "[Report](https://user:password@example.com/report?token=PRIVATE#PRIVATE)"
            ),
            "[Report](example.com/report)"
        );
        assert_eq!(narration_detail("abc\u{202e}txt"), "abctxt");
        assert_eq!(
            resource_detail("https://user:PRIVATE@example.com/report?token=PRIVATE"),
            "example.com/report"
        );
    }

    #[test]
    fn action_selects_only_safe_fields_and_handles_all_phases() {
        for (phase, expected) in [
            (ToolNarrationPhase::Started, "Reading"),
            (ToolNarrationPhase::Waiting, "Reading"),
            (ToolNarrationPhase::Completed, "Read"),
            (ToolNarrationPhase::Failed, "Could not read"),
        ] {
            assert_eq!(
                narrate_labeled_action(
                    &json!({"name":"  ","title":"Report","token":"PRIVATE","content":"PRIVATE"}),
                    phase,
                    None,
                    ("Reading", "Read", "Could not read"),
                    ("Читаю", "Прочитав", "Не вдалося прочитати"),
                    &["token", "name", "title"]
                ),
                format!("{expected}: Report")
            );
            assert_eq!(
                narrate_labeled_action(
                    &json!({"content":"PRIVATE"}),
                    phase,
                    None,
                    ("Reading", "Read", "Could not read"),
                    ("Читаю", "Прочитав", "Не вдалося прочитати"),
                    &["name"]
                ),
                expected
            );
        }
        assert_eq!(
            narrate_labeled_action(
                &json!({}),
                ToolNarrationPhase::Completed,
                Some("uk-UA"),
                ("Reading", "Read", "Could not read"),
                ("Читаю", "Прочитав", "Не вдалося прочитати"),
                &[]
            ),
            "Прочитав"
        );
    }
}
