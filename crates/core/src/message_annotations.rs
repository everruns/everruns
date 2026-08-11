//! Synthetic message-annotation hygiene applied by the kernel.
//!
//! The `message_metadata` capability (in `everruns-builtins`) prepends
//! `[time …]` to the model's view of a message. Models echo that prefix back
//! into their replies (EVE-710), so the kernel strips it before persisting
//! assistant text — which means the stripper has to live here, below the
//! capability that injects it (EVE-884).

/// Remove synthetic `[time <rfc3339>]` annotations from the start of model
/// output before it is persisted.
///
/// This removes only the exact synthetic pattern — one or more leading
/// `[time <timestamp>]` segments, each followed by an optional single space —
/// validating the bracket contents as a real RFC3339 timestamp so legitimate
/// user-authored text like `[time to go]` is left untouched. Only leading
/// occurrences are removed; a bracketed timestamp later in the text is treated
/// as authored content and preserved.
pub fn strip_leading_timestamp_annotations(text: &str) -> String {
    let mut rest = text;
    while let Some(after) = strip_one_timestamp_annotation(rest) {
        rest = after;
    }
    rest.to_string()
}

/// Strip a single leading `[time <rfc3339>]` (plus one optional following space)
/// from `text`, returning the remainder when it matches the synthetic format.
fn strip_one_timestamp_annotation(text: &str) -> Option<&str> {
    const PREFIX: &str = "[time ";
    let rest = text.strip_prefix(PREFIX)?;
    let close = rest.find(']')?;
    // Only strip when the bracket holds a real RFC3339 timestamp — the exact
    // synthetic format produced by `MessageMetadataField::Timestamp::render`.
    chrono::DateTime::parse_from_rfc3339(&rest[..close]).ok()?;
    let after = &rest[close + 1..];
    Some(after.strip_prefix(' ').unwrap_or(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_repeated_leading_timestamp_annotations() {
        let text = "[time 2026-06-11T09:15:42Z] [time 2026-06-11T09:16:42Z] hello";
        assert_eq!(strip_leading_timestamp_annotations(text), "hello");
    }

    #[test]
    fn keeps_authored_bracket_text_and_later_timestamps() {
        assert_eq!(
            strip_leading_timestamp_annotations("[time to go] now"),
            "[time to go] now"
        );
        assert_eq!(
            strip_leading_timestamp_annotations("done at [time 2026-06-11T09:15:42Z]"),
            "done at [time 2026-06-11T09:15:42Z]"
        );
    }
}
