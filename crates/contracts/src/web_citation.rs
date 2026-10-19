//! Provider-native web citations, carried from a chat driver to the
//! `citation_web` feed.
//!
//! Decision: drivers do not attach `TextAnnotation`s themselves. A driver
//! reports what the provider cited on
//! [`LlmCompletionMetadata::web_citations`](crate::stream_event::LlmCompletionMetadata::web_citations),
//! the reason atom hands the list to the post-generation annotation hooks, and
//! only the `citation_web` capability turns it into annotations. So native
//! citations appear exactly when that capability is on, with the same origin,
//! matching, and verification path as citations of `web_search` function
//! results, and no driver needs to know about capabilities.
//!
//! Spans: every provider counts differently (Anthropic cites whole text
//! blocks, OpenAI Responses uses Unicode scalar indices into one output text,
//! Gemini UTF-8 byte offsets). The driver converts with the [`CitedSpan`]
//! constructors into char offsets into the assistant text it streamed (all its
//! `TextDelta`s, concatenated) and keeps the cited text. The feed re-checks
//! that text against the final message, which a response filter may have
//! changed, and drops a citation whose span no longer matches rather than
//! attach it to the wrong words. A constructor returns `None` for a span it
//! cannot map exactly.

/// One source the provider cited for part of its answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebCitation {
    /// The cited page, as the provider gave it.
    pub url: String,
    pub title: Option<String>,
    /// Text from the page that backs the claim (Anthropic `cited_text`), when
    /// the provider returns it.
    pub snippet: Option<String>,
    /// The part of the answer the citation covers. `None` when the provider
    /// gave no usable range; the feed then looks for the URL in the answer.
    pub span: Option<CitedSpan>,
}

/// A char range of the streamed assistant text, with the text it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitedSpan {
    /// Start, in chars (Unicode scalar values), inclusive.
    pub start: usize,
    /// End, in chars, exclusive.
    pub end: usize,
    /// The covered text, `text.chars().skip(start).take(end - start)`.
    pub text: String,
}

impl CitedSpan {
    /// The span `[start, end)` of `text`, counted in chars. `None` when it is
    /// empty, reversed, or out of range.
    pub fn from_chars(text: &str, start: usize, end: usize) -> Option<Self> {
        if start >= end {
            return None;
        }
        let covered: String = text.chars().skip(start).take(end - start).collect();
        if covered.chars().count() != end - start {
            return None;
        }
        Self::covering(start, end, covered)
    }

    /// The span `[start, end)` of `text`, counted in UTF-8 bytes. `None`
    /// unless both ends fall on char boundaries inside `text`.
    pub fn from_bytes(text: &str, start: usize, end: usize) -> Option<Self> {
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return None;
        }
        let covered = text.get(start..end)?;
        let start_chars = text[..start].chars().count();
        Self::covering(start_chars, start_chars + covered.chars().count(), covered)
    }

    /// The same span in a text that `chars` chars precede, for a part
    /// (content block, output item) that is not the first of the answer.
    pub fn offset(mut self, chars: usize) -> Self {
        self.start += chars;
        self.end += chars;
        self
    }

    fn covering(start: usize, end: usize, text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        (!text.trim().is_empty()).then_some(Self { start, end, text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_spans_count_scalar_values() {
        let text = "Zürich 😀 is big.";
        let span = CitedSpan::from_chars(text, 9, 15).unwrap();
        assert_eq!(span.text, "is big");
        assert_eq!((span.start, span.end), (9, 15));
        assert_eq!(CitedSpan::from_chars(text, 0, 1).unwrap().text, "Z");
        assert!(CitedSpan::from_chars(text, 3, 3).is_none());
        assert!(CitedSpan::from_chars(text, 4, 2).is_none());
        assert!(CitedSpan::from_chars(text, 10, 99).is_none());
        assert!(
            CitedSpan::from_chars(text, 6, 7).is_none(),
            "whitespace only"
        );
    }

    #[test]
    fn byte_spans_convert_to_chars_and_reject_split_characters() {
        let text = "Zürich 😀 is big.";
        // "ü" is 2 bytes and "😀" 4, so "is" starts at byte 13 and char 9.
        let start = text.find("is").unwrap();
        assert_eq!(start, 13);
        let span = CitedSpan::from_bytes(text, start, start + 6).unwrap();
        assert_eq!(
            (span.start, span.end, span.text.as_str()),
            (9, 15, "is big")
        );
        assert_eq!(CitedSpan::from_bytes(text, 0, 3).unwrap().text, "Zü");
        assert!(CitedSpan::from_bytes(text, 0, 2).is_none(), "splits ü");
        assert!(
            CitedSpan::from_bytes(text, 8, 10).is_none(),
            "splits the emoji"
        );
        assert!(CitedSpan::from_bytes(text, 13, 999).is_none());
        assert!(CitedSpan::from_bytes(text, 5, 5).is_none());
    }

    #[test]
    fn offset_shifts_both_ends() {
        let span = CitedSpan::from_chars("abc", 1, 3).unwrap().offset(10);
        assert_eq!((span.start, span.end, span.text.as_str()), (11, 13, "bc"));
    }
}
