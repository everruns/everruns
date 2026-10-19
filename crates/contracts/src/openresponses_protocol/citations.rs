//! `url_citation` annotations of a Responses API answer, as
//! [`WebCitation`]s for the `citation_web` feed (see [`crate::web_citation`]).
//!
//! OpenAI's hosted `web_search` and OpenRouter's `openrouter:web_search` both
//! annotate `output_text` parts with `url_citation`s whose `start_index` /
//! `end_index` count Unicode scalar values (chars) of that part's text. The
//! streamed answer is every `output_text` delta in output order, so a part
//! starts where the previous parts' chars end. OpenRouter's chat-completions
//! shape nests the fields under `url_citation` and may add the page `content`;
//! both shapes are read.

use serde_json::Value;

use crate::web_citation::{CitedSpan, WebCitation};

/// The citations of a terminal response's `output` array.
pub(crate) fn web_citations(output: Option<&Value>) -> Vec<WebCitation> {
    let mut out = Vec::new();
    let mut offset = 0;
    let messages = output
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"));
    for message in messages {
        let parts = message.get("content").and_then(Value::as_array);
        for part in parts.into_iter().flatten() {
            if part.get("type").and_then(Value::as_str) != Some("output_text") {
                continue;
            }
            let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
            let annotations = part.get("annotations").and_then(Value::as_array);
            for annotation in annotations.into_iter().flatten() {
                if let Some(citation) = url_citation(annotation, text, offset) {
                    out.push(citation);
                }
            }
            offset += text.chars().count();
        }
    }
    out
}

fn url_citation(annotation: &Value, text: &str, offset: usize) -> Option<WebCitation> {
    if annotation.get("type").and_then(Value::as_str) != Some("url_citation") {
        return None;
    }
    let fields = annotation.get("url_citation").unwrap_or(annotation);
    let string = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let index = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
    };
    let span = match (index("start_index"), index("end_index")) {
        (Some(start), Some(end)) => CitedSpan::from_chars(text, start, end),
        _ => None,
    };
    Some(WebCitation {
        url: string("url")?,
        title: string("title"),
        snippet: string("content"),
        span: span.map(|span| span.offset(offset)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spans_count_chars_across_output_texts() {
        // "Grüße 😀" is 7 chars and 12 bytes: indices are chars.
        let first = "Grüße 😀 ([a.io](https://a.io)).";
        let second = "Zweitens ([b.io](https://b.io/x)).";
        let output = json!([
            { "type": "web_search_call", "id": "ws_1", "status": "completed" },
            { "type": "message", "role": "assistant", "content": [
                { "type": "output_text", "text": first, "annotations": [
                    { "type": "url_citation", "url": "https://a.io", "title": "A",
                      "start_index": 8, "end_index": 30 } ] } ] },
            { "type": "message", "role": "assistant", "content": [
                { "type": "output_text", "text": second, "annotations": [
                    { "type": "url_citation", "url_citation": { "url": "https://b.io/x",
                      "title": "B", "content": "Page text", "start_index": 9, "end_index": 33 } },
                    { "type": "file_citation", "file_id": "f" },
                    { "type": "url_citation", "url": "https://c.io",
                      "start_index": 5, "end_index": 999 } ] } ] }
        ]);
        let cites = web_citations(Some(&output));
        assert_eq!(cites.len(), 3);
        let streamed = format!("{first}{second}");
        let check = |citation: &WebCitation, expected: &str| {
            let span = citation.span.as_ref().unwrap();
            let covered: String = streamed
                .chars()
                .skip(span.start)
                .take(span.end - span.start)
                .collect();
            assert_eq!(covered, expected);
            assert_eq!(span.text, expected);
        };
        check(&cites[0], "([a.io](https://a.io))");
        assert_eq!(cites[0].title.as_deref(), Some("A"));
        check(&cites[1], "([b.io](https://b.io/x))");
        assert_eq!(cites[1].snippet.as_deref(), Some("Page text"));
        // Out of range: kept without a span, to be placed by its URL.
        assert_eq!(cites[2].url, "https://c.io");
        assert_eq!(cites[2].span, None);
        assert!(web_citations(None).is_empty());
    }
}
