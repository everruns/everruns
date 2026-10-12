//! `citation_web` capability — claim-level citations from web search.
//!
//! The web search feed of Citations. Like
//! `citation_retrieval` it contributes no tools, only a post-generation
//! annotation hook, and never rewrites the answer. Two inputs:
//!
//! - **Function results.** The `web_search` tool tells the model to cite each
//!   claim with a result's `url` or `id`. The hook finds those URLs and `w…`
//!   ids in the answer and resolves them against the `web_search` results in
//!   the turn's context. Matching is exact, never fuzzy: an id equal to a
//!   result's, or a URL equal to a result's after normalization (scheme and
//!   host case, fragment, trailing slash). The citation covers the sentence
//!   holding the URL or id, the span unit `citation_retrieval` uses; a
//!   citation standing alone after its sentence ("… shipped. (source: url)")
//!   attaches to that sentence.
//! - **Native results.** Drivers report the provider's own citations
//!   (Anthropic `web_search_result_location`, OpenAI Responses and OpenRouter
//!   `url_citation`, Gemini grounding supports) on the completion, as char
//!   spans of the streamed text with the text they cover; see
//!   [`everruns_contracts::web_citation`]. The hook keeps a span only while
//!   that text still sits there in the final answer (or occurs exactly once
//!   elsewhere in it), and otherwise drops the citation rather than mark the
//!   wrong words. A citation without a span is placed by its URL, as above.
//!
//! Every annotation has `origin` `citation_web`, the capability id, as the
//! spec requires of each feed; `external_id` is the result id (`w…`), derived
//! from the URL for native citations too, so one page has one id. Results of
//! `brave_web_search` (the Brave-specific capability) share the `results[].url`
//! shape and are resolved by URL; DuckDuckGo instant answers are not search
//! results and are skipped.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use everruns_contracts::runtime::web_search::{WEB_SEARCH_TOOL_NAME, web_result_id};
use everruns_contracts::web_citation::WebCitation;
use everruns_core::annotation_hook::{
    AnnotationContext, AnnotationResult, PostGenerationAnnotationHook, span_text,
};
use everruns_core::capabilities::Capability;
use everruns_core::capability_types::CapabilityStatus;
use everruns_core::message::{AnnotationSource, ContentPart, RuntimeMessage, TextAnnotation};

/// Canonical capability id, and the `origin` of every annotation it makes.
pub const CITATION_WEB_CAPABILITY_ID: &str = "citation_web";

/// Tools whose results are web search hits with a `results[].url`.
const WEB_TOOLS: &[&str] = &[WEB_SEARCH_TOOL_NAME, "brave_web_search"];

/// Snippets longer than this are cut for display.
const MAX_SNIPPET_CHARS: usize = 500;

/// The web search citation feed.
pub struct CitationWebCapability;

impl Capability for CitationWebCapability {
    fn id(&self) -> &str {
        CITATION_WEB_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Web citations"
    }

    fn description(&self) -> &str {
        "Attach claim-level citations to the answer from web search results, \
         from the web_search tool or the model's own search, so each cited \
         sentence links to its source."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("quote")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    /// Surfaces the citation UI when any citation feed is active.
    fn features(&self) -> Vec<&'static str> {
        vec!["citations"]
    }

    fn post_output_annotation_hooks_with_config(
        &self,
        _config: &Value,
    ) -> Vec<Arc<dyn PostGenerationAnnotationHook>> {
        vec![Arc::new(WebAnnotationHook)]
    }
}

struct WebAnnotationHook;

#[async_trait]
impl PostGenerationAnnotationHook for WebAnnotationHook {
    fn id(&self) -> &str {
        CITATION_WEB_CAPABILITY_ID
    }

    async fn annotate(&self, ctx: &AnnotationContext<'_>) -> AnnotationResult {
        let results = WebResults::from_messages(ctx.messages);
        let mut annotations = cite_function_results(ctx.message_text, &results);
        annotations.extend(cite_native(ctx.message_text, ctx.web_citations, &results));
        let mut seen = HashSet::new();
        annotations.retain(|a| seen.insert((a.start, a.end, a.source.uri.clone())));
        AnnotationResult {
            annotations,
            rewritten_text: None,
        }
    }
}

/// One search result the model saw.
#[derive(Debug, Clone)]
struct WebResult {
    id: String,
    url: String,
    title: Option<String>,
    snippet: Option<String>,
}

/// The turn's search results, by id and by normalized URL. The first result
/// for a page wins; later searches return the same page under the same id.
#[derive(Default)]
struct WebResults {
    by_id: HashMap<String, WebResult>,
    by_url: HashMap<String, WebResult>,
}

impl WebResults {
    fn from_messages(messages: &[RuntimeMessage]) -> Self {
        let mut tool_names: HashMap<&str, &str> = HashMap::new();
        for msg in messages {
            for part in &msg.content {
                if let ContentPart::ToolCall(call) = part {
                    tool_names.insert(call.id.as_str(), call.name.as_str());
                }
            }
        }
        let mut out = Self::default();
        for msg in messages {
            for part in &msg.content {
                let ContentPart::ToolResult(result) = part else {
                    continue;
                };
                if !tool_names
                    .get(result.tool_call_id.as_str())
                    .is_some_and(|name| WEB_TOOLS.contains(name))
                {
                    continue;
                }
                let entries = result
                    .result
                    .as_ref()
                    .and_then(|value| value.get("results"))
                    .and_then(Value::as_array);
                for entry in entries.into_iter().flatten() {
                    if let Some(result) = web_result(entry) {
                        out.insert(result);
                    }
                }
            }
        }
        out
    }

    fn insert(&mut self, result: WebResult) {
        let Some(key) = normalize_url(&result.url) else {
            return;
        };
        self.by_id
            .entry(result.id.clone())
            .or_insert_with(|| result.clone());
        self.by_url.entry(key).or_insert(result);
    }

    fn by_url(&self, url: &str) -> Option<&WebResult> {
        self.by_url.get(&normalize_url(url)?)
    }
}

/// A `web_search` hit (`id`, `url`, `title`, `snippet` or `excerpt`) or a
/// Brave one (`url`, `title`, `description`, no id).
fn web_result(entry: &Value) -> Option<WebResult> {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let url = text("url")?.to_string();
    Some(WebResult {
        id: text("id").map_or_else(|| web_result_id(&url), str::to_string),
        title: text("title").map(str::to_string),
        snippet: ["snippet", "excerpt", "description"]
            .into_iter()
            .find_map(text)
            .map(|snippet| snippet.chars().take(MAX_SNIPPET_CHARS).collect()),
        url,
    })
}

/// Annotations for the result URLs and ids the answer cites.
fn cite_function_results(text: &str, results: &WebResults) -> Vec<TextAnnotation> {
    if results.by_id.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let urls = find_urls(&chars);
    let mut annotations = Vec::new();
    for (start, end, url) in &urls {
        if let Some(result) = results.by_url(url) {
            let (from, to) = claim_span(&chars, *start, *end);
            annotations.push(annotation(from, to, result, None));
        }
    }
    for (start, end, id) in find_ids(&chars) {
        let inside_url = urls.iter().any(|(s, e, _)| start >= *s && end <= *e);
        if let Some(result) = results.by_id.get(&id).filter(|_| !inside_url) {
            let (from, to) = claim_span(&chars, start, end);
            annotations.push(annotation(from, to, result, None));
        }
    }
    annotations
}

/// Annotations for the citations the provider returned itself.
fn cite_native(text: &str, citations: &[WebCitation], results: &WebResults) -> Vec<TextAnnotation> {
    if citations.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let urls = find_urls(&chars);
    let mut annotations = Vec::new();
    for citation in citations {
        let span = match &citation.span {
            Some(span) => locate(text, &chars, span.start, span.end, &span.text),
            None => urls
                .iter()
                .find(|(_, _, url)| normalize_url(url) == normalize_url(&citation.url))
                .map(|(start, end, _)| claim_span(&chars, *start, *end)),
        };
        let Some((start, end)) = span.and_then(|(s, e)| trim_span(&chars, s, e)) else {
            continue;
        };
        let known = results.by_url(&citation.url);
        let result = WebResult {
            id: known.map_or_else(|| web_result_id(&citation.url), |r| r.id.clone()),
            url: citation.url.clone(),
            title: citation
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .or_else(|| known.and_then(|r| r.title.clone())),
            snippet: known.and_then(|r| r.snippet.clone()),
        };
        let snippet = citation
            .snippet
            .as_deref()
            .map(str::trim)
            .filter(|snippet| !snippet.is_empty())
            .map(|snippet| snippet.chars().take(MAX_SNIPPET_CHARS).collect());
        annotations.push(annotation(start, end, &result, snippet));
    }
    annotations
}

fn annotation(
    start: usize,
    end: usize,
    result: &WebResult,
    snippet: Option<String>,
) -> TextAnnotation {
    TextAnnotation {
        start,
        end,
        origin: CITATION_WEB_CAPABILITY_ID.to_string(),
        source: AnnotationSource {
            uri: result.url.clone(),
            title: result.title.clone(),
            snippet: snippet.or_else(|| result.snippet.clone()),
            location: None,
        },
        external_id: Some(result.id.clone()),
        verified: None,
    }
}

/// Where the provider's span sits in the final text: in place when the text
/// there is unchanged, else at the cited text's only occurrence. `None` when
/// neither holds.
fn locate(
    text: &str,
    chars: &[char],
    start: usize,
    end: usize,
    cited: &str,
) -> Option<(usize, usize)> {
    if end <= chars.len() && span_text(text, start, end) == cited {
        return Some((start, end));
    }
    let mut matches = text.match_indices(cited);
    let (byte, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let start = text[..byte].chars().count();
    Some((start, start + cited.chars().count()))
}

/// The span without surrounding whitespace; `None` when nothing remains.
fn trim_span(chars: &[char], mut start: usize, mut end: usize) -> Option<(usize, usize)> {
    while start < end && chars[start].is_whitespace() {
        start += 1;
    }
    while end > start && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    (start < end).then_some((start, end))
}

/// `https://Tokio.RS/blog/#top` and `https://tokio.rs/blog` are the same
/// page: scheme and host compare case-insensitively, the fragment and a
/// trailing slash do not count. Path and query compare exactly. `None` for
/// anything but an http(s) URL.
fn normalize_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or_default();
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(split);
    if host.is_empty() {
        return None;
    }
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    let mut out = format!(
        "{scheme}://{}{}",
        host.to_ascii_lowercase(),
        path.trim_end_matches('/')
    );
    if !query.is_empty() {
        out.push('?');
        out.push_str(query);
    }
    Some(out)
}

/// Every `http(s)://` URL in the text as `(start, end, url)` char spans. A URL
/// ends at whitespace, at a quote or bracket that closes markdown or HTML
/// around it, or at a `)` it did not open; trailing sentence punctuation is
/// not part of it.
fn find_urls(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let rest_len = ["https://", "http://"]
            .into_iter()
            .find(|prefix| starts_with_ignore_case(&chars[i..], prefix))
            .map(str::len);
        let bounded = i == 0 || !chars[i - 1].is_alphanumeric();
        let Some(prefix_len) = rest_len.filter(|_| bounded) else {
            i += 1;
            continue;
        };
        let mut end = i + prefix_len;
        let mut depth = 0usize;
        while end < chars.len() {
            let c = chars[end];
            if c.is_whitespace()
                || matches!(
                    c,
                    '<' | '>' | '"' | '\'' | '`' | ']' | '[' | '{' | '}' | '|'
                )
            {
                break;
            }
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            end += 1;
        }
        while end > i + prefix_len
            && matches!(
                chars[end - 1],
                '.' | ',' | ';' | ':' | '!' | '?' | '*' | '_'
            )
        {
            end -= 1;
        }
        if end > i + prefix_len {
            out.push((i, end, chars[i..end].iter().collect()));
        }
        i = end.max(i + 1);
    }
    out
}

fn starts_with_ignore_case(chars: &[char], prefix: &str) -> bool {
    prefix.chars().count() <= chars.len()
        && prefix
            .chars()
            .zip(chars)
            .all(|(p, c)| c.eq_ignore_ascii_case(&p))
}

/// Every result id (`w` and 6 lowercase hex digits, standing alone) in the
/// text as `(start, end, id)` char spans.
fn find_ids(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for i in 0..chars.len() {
        let end = i + 7;
        if chars[i] != 'w' || end > chars.len() {
            continue;
        }
        let hex = chars[i + 1..end]
            .iter()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'));
        let before = i == 0 || !chars[i - 1].is_alphanumeric();
        let after = end == chars.len() || !chars[end].is_alphanumeric();
        if hex && before && after {
            out.push((i, end, chars[i..end].iter().collect()));
        }
    }
    out
}

/// The claim a citation at `[start, end)` supports: the sentence holding it.
/// When the sentence is nothing but the citation (its URL or id, brackets and
/// a label like "source"), the citation trails its claim, so the previous
/// sentence is the claim.
fn claim_span(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    const LABELS: &[&str] = &["source", "sources", "see", "via", "ref", "from"];
    let (from, to) = sentence_around(chars, start, end);
    let rest: String = chars[from..start].iter().chain(&chars[end..to]).collect();
    let only_citation = rest
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .all(|word| LABELS.contains(&word.to_lowercase().as_str()));
    if !only_citation {
        return (from, to);
    }
    // The previous sentence ends at the last non-blank char before this one.
    let Some(last) = chars[..from].iter().rposition(|c| !c.is_whitespace()) else {
        return (from, to);
    };
    let (prev_from, _) = sentence_around(chars, last, last);
    if chars[prev_from..=last].iter().any(|c| c.is_alphanumeric()) {
        (prev_from, last + 1)
    } else {
        (from, to)
    }
}

/// The sentence holding `[start, end)`: from after the previous sentence end
/// (`.`, `!` or `?` before whitespace, or a line break) to its own end,
/// without surrounding whitespace. Dots inside URLs and numbers do not end a
/// sentence, since whitespace must follow.
fn sentence_around(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    let ends_sentence = |i: usize| {
        chars[i] == '\n'
            || (matches!(chars[i], '.' | '!' | '?')
                && chars.get(i + 1).is_none_or(|c| c.is_whitespace()))
    };
    let from = (0..start)
        .rev()
        .find(|&i| ends_sentence(i))
        .map_or(0, |i| i + 1);
    let to = (end..chars.len())
        .find(|&i| ends_sentence(i))
        .map_or(chars.len(), |i| if chars[i] == '\n' { i } else { i + 1 });
    trim_span(chars, from, to).unwrap_or((start, end))
}

#[cfg(test)]
#[path = "citation_web_tests.rs"]
mod tests;
