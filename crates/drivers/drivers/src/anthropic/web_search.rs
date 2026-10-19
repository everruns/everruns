//! Anthropic's web search server tool as native web search
//! ([`everruns_contracts::native_web_search`]).
//!
//! When the call asks for native search, the model takes the tool and
//! Anthropic can apply the agent's domain filters, the `web_search` function
//! tool is replaced by the `web_search_20250305` server tool of the same name
//! (see `NativeWebSearch::anthropic_tool`). Anthropic runs the searches inside
//! the generation: the response carries `server_tool_use` blocks named
//! `web_search` and `web_search_tool_result` blocks, which are never client
//! tool calls. Each surfaces as a hosted call on the stream, and
//! `usage.server_tool_use.web_search_requests` counts the billed searches.
//!
//! Replay: the driver keeps every response's raw blocks and sends them back
//! verbatim, which carries each result's `encrypted_content` and each
//! citation's `encrypted_index`, as the API requires. Two cases would make
//! the API reject that history, and [`rewrite_messages`] removes the search
//! blocks there instead of failing every later call of the session:
//!
//! - a call without the server tool (the plan, the organization's settings or
//!   the model changed): the API rejects search blocks it cannot pair with a
//!   tool, so every search block goes, with the citations that point into
//!   their results;
//! - a `server_tool_use` without its result. The API leaves one open when
//!   Claude also called a client tool (`stop_reason: tool_use`; the search
//!   runs once the tool results arrive) or when it paused a long turn
//!   (`pause_turn`). Only the first is still waiting, and only while the
//!   results of that very response are what follows it; anything else
//!   unpaired is dropped, since the API rejects a search call without a
//!   result once the assistant turn is over.
//!
//! Citations: a text block that cites search results carries
//! `web_search_result_location` citations, and the block is the claim.
//! [`citations`] reports them for the `citation_web` feed with the block's
//! char span in the streamed answer (the text blocks' deltas, in order).
//!
//! Decision: a paused turn is not continued. The driver ends the generation
//! with what the model wrote so far (`pause_turn` maps to `stop`), and the
//! default search cap keeps generations well under the server loop's limit.

use std::collections::HashSet;

use everruns_contracts::driver_registry::{
    HostedToolCall, HostedToolCallStatus, LlmCallConfig, LlmStreamEvent,
};
use everruns_contracts::native_web_search::{
    NativeWebSearch, WEB_SEARCH_FUNCTION_TOOL, anthropic_has_native_web_search,
};
use everruns_contracts::web_citation::{CitedSpan, WebCitation};
use serde_json::Value;

const SERVER_TOOL_USE: &str = "server_tool_use";
const RESULT_BLOCK: &str = "web_search_tool_result";
const RESULT_ERROR: &str = "web_search_tool_result_error";
const CITATION: &str = "web_search_result_location";

/// The server tool entry for this call, when native search is requested, the
/// model has it, and Anthropic can apply the domain filters.
pub(crate) fn server_tool(config: &LlmCallConfig, wire_model: &str) -> Option<Value> {
    if !anthropic_has_native_web_search(wire_model) {
        return None;
    }
    NativeWebSearch::requested(config)?.anthropic_tool()
}

/// Whether a raw block is a `server_tool_use` of the web search tool.
fn is_search_call(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some(SERVER_TOOL_USE)
        && block.get("name").and_then(Value::as_str) == Some(WEB_SEARCH_FUNCTION_TOOL)
}

fn is_search_result(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some(RESULT_BLOCK)
}

fn block_id<'a>(block: &'a Value, field: &str) -> Option<&'a str> {
    block.get(field).and_then(Value::as_str)
}

fn role(message: &Value) -> Option<&str> {
    message.get("role").and_then(Value::as_str)
}

/// A user message that holds only `tool_result` blocks.
fn is_tool_results(message: &Value) -> bool {
    role(message) == Some("user")
        && message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks.iter().all(|block| {
                        block.get("type").and_then(Value::as_str) == Some("tool_result")
                    })
            })
}

/// Make replayed web search blocks acceptable to this request; see the
/// module docs. `active` is whether this request carries the server tool.
pub(crate) fn rewrite_messages(messages: &mut Vec<Value>, active: bool) {
    let assistant_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| role(message) == Some("assistant"))
        .map(|(index, _)| index)
        .collect();
    let mut emptied = Vec::new();
    for (position, &index) in assistant_indices.iter().enumerate() {
        let next_assistant = assistant_indices.get(position + 1).copied();
        // Results of this message's searches: here, or at the start of the
        // next assistant message when the search waited on client tools.
        let mut resolved: HashSet<String> = HashSet::new();
        for source in std::iter::once(index).chain(next_assistant) {
            if let Some(blocks) = messages[source].get("content").and_then(Value::as_array) {
                resolved.extend(
                    blocks
                        .iter()
                        .filter(|block| is_search_result(block))
                        .filter_map(|block| block_id(block, "tool_use_id"))
                        .map(str::to_string),
                );
            }
        }
        // Still waiting: the last assistant message, followed only by the
        // tool results of its client calls.
        let waiting = next_assistant.is_none()
            && index + 1 < messages.len()
            && messages[index + 1..].iter().all(is_tool_results);
        let Some(blocks) = messages[index]
            .get_mut("content")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        let had_blocks = !blocks.is_empty();
        if active {
            blocks.retain(|block| {
                !is_search_call(block)
                    || waiting
                    || block_id(block, "id").is_some_and(|id| resolved.contains(id))
            });
        } else {
            blocks.retain(|block| !is_search_call(block) && !is_search_result(block));
            for block in blocks.iter_mut() {
                strip_search_citations(block);
            }
        }
        if had_blocks && blocks.is_empty() {
            emptied.push(index);
        }
    }
    // An assistant message left with no content is rejected; the API merges
    // the consecutive user turns its removal leaves.
    for index in emptied.into_iter().rev() {
        messages.remove(index);
    }
}

/// Drop citations that point into removed search results.
fn strip_search_citations(block: &mut Value) {
    let Some(fields) = block.as_object_mut() else {
        return;
    };
    let Some(citations) = fields.get_mut("citations").and_then(Value::as_array_mut) else {
        return;
    };
    citations.retain(|citation| citation.get("type").and_then(Value::as_str) != Some(CITATION));
    if citations.is_empty() {
        fields.remove("citations");
    }
}

/// The hosted call a raw `content_block_start` block reports: a search the
/// model starts, or its result. `query` names the search a result answers.
pub(crate) fn hosted_call_at_start(
    block: &Value,
    query: impl FnOnce(&str) -> Option<String>,
) -> Option<LlmStreamEvent> {
    if is_search_call(block) {
        return Some(hosted_call(
            block_id(block, "id")?,
            HostedToolCallStatus::InProgress,
            search_query(block),
        ));
    }
    if !is_search_result(block) {
        return None;
    }
    let id = block_id(block, "tool_use_id")?;
    let failed = block
        .get("content")
        .and_then(|content| content.get("type"))
        .and_then(Value::as_str)
        == Some(RESULT_ERROR);
    let status = if failed {
        HostedToolCallStatus::Failed
    } else {
        HostedToolCallStatus::Completed
    };
    Some(hosted_call(id, status, query(id)))
}

/// The in-progress call again once its query has streamed in, at
/// `content_block_stop`. A call that streamed no input gets the empty input
/// the API expects on replay.
pub(crate) fn hosted_call_at_stop(block: &mut Value) -> Option<LlmStreamEvent> {
    if !is_search_call(block) {
        return None;
    }
    if let Some(fields) = block.as_object_mut() {
        fields
            .entry("input")
            .or_insert_with(|| Value::Object(Default::default()));
    }
    let query = search_query(block)?;
    Some(hosted_call(
        block_id(block, "id")?,
        HostedToolCallStatus::InProgress,
        Some(query),
    ))
}

/// The query of a raw `server_tool_use` block, once known.
pub(crate) fn search_query(block: &Value) -> Option<String> {
    block
        .pointer("/input/query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .map(|query| query.chars().take(200).collect())
}

fn hosted_call(id: &str, status: HostedToolCallStatus, summary: Option<String>) -> LlmStreamEvent {
    LlmStreamEvent::HostedToolCall(HostedToolCall {
        id: id.to_string(),
        tool: WEB_SEARCH_FUNCTION_TOOL.to_string(),
        status,
        summary,
    })
}

/// Billed searches: `usage.server_tool_use.web_search_requests`, or the
/// successful results in the response when usage reports fewer (the two
/// describe the same searches, so the larger wins rather than the sum).
pub(crate) fn search_count(reported: u32, content: &[Value]) -> u32 {
    let results = content
        .iter()
        .filter(|block| is_search_result(block))
        .filter(|block| block.get("content").is_some_and(Value::is_array))
        .count();
    reported.max(u32::try_from(results).unwrap_or(u32::MAX))
}

/// The web search citations of a response's raw blocks, each spanning the
/// text block that carries it. The streamed answer is every text block's text
/// in block order, so a block starts where the previous ones' chars end.
pub(crate) fn citations(content: &[Value]) -> Vec<WebCitation> {
    let mut out = Vec::new();
    let mut offset = 0;
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let chars = text.chars().count();
        let cites = block.get("citations").and_then(Value::as_array);
        for citation in cites.into_iter().flatten() {
            let field = |key: &str| citation.get(key).and_then(Value::as_str);
            let Some(url) = field("url").filter(|_| field("type") == Some(CITATION)) else {
                continue;
            };
            out.push(WebCitation {
                url: url.to_string(),
                title: field("title").map(str::to_string),
                snippet: field("cited_text").map(str::to_string),
                span: CitedSpan::from_chars(text, 0, chars).map(|span| span.offset(offset)),
            });
        }
        offset += chars;
    }
    out
}

/// `usage.server_tool_use.web_search_requests` from a usage payload.
pub(crate) fn reported_searches(server_tool_use: Option<&Value>) -> Option<u32> {
    server_tool_use?
        .get("web_search_requests")
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::ToolDefinition;
    use serde_json::json;

    fn call(id: &str) -> Value {
        json!({ "type": "server_tool_use", "id": id, "name": "web_search",
                "input": { "query": "rust release" } })
    }

    fn result(id: &str) -> Value {
        json!({ "type": "web_search_tool_result", "tool_use_id": id, "content": [
            { "type": "web_search_result", "url": "https://blog.rust-lang.org",
              "title": "Rust", "encrypted_content": "EqgfCio" } ] })
    }

    fn cited_text() -> Value {
        json!({ "type": "text", "text": "Rust 1.95 is out.", "citations": [
            { "type": "web_search_result_location", "url": "https://blog.rust-lang.org",
              "title": "Rust", "encrypted_index": "Eo8B", "cited_text": "1.95" } ] })
    }

    #[test]
    fn server_tool_needs_the_option_and_a_model_with_search() {
        let mut config = LlmCallConfig::new("claude-haiku-5-5");
        config.tools = vec![ToolDefinition::function("web_search", "", json!({}))];
        assert_eq!(server_tool(&config, "claude-haiku-5-5"), None);
        let (key, value) = NativeWebSearch::default().to_driver_option();
        config.driver_options.insert(key, value);
        assert_eq!(
            server_tool(&config, "claude-haiku-5-5").unwrap()["type"],
            "web_search_20250305"
        );
        assert_eq!(server_tool(&config, "claude-3-5-sonnet-20241022"), None);
    }

    #[test]
    fn active_replay_keeps_paired_searches_verbatim() {
        let assistant = json!({ "role": "assistant", "content": [
            call("srv_1"), result("srv_1"), cited_text() ] });
        let mut messages = vec![
            json!({ "role": "user", "content": "news?" }),
            assistant.clone(),
            json!({ "role": "user", "content": "more" }),
        ];
        rewrite_messages(&mut messages, true);
        assert_eq!(messages[1], assistant);
    }

    #[test]
    fn active_replay_keeps_a_search_waiting_on_client_tools() {
        // Claude searched and called a client tool at once: the search runs
        // when the tool results arrive, and its result opens the next answer.
        let waiting = json!({ "role": "assistant", "content": [
            call("srv_1"),
            { "type": "tool_use", "id": "toolu_1", "name": "get_time", "input": {} } ] });
        let results = json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "noon" } ] });
        let mut messages = vec![
            json!({ "role": "user", "content": "q" }),
            waiting.clone(),
            results.clone(),
        ];
        rewrite_messages(&mut messages, true);
        assert_eq!(messages[1], waiting);

        // Answered in the next assistant message: the pair spans two messages.
        let answered = json!({ "role": "assistant", "content": [ result("srv_1"), cited_text() ] });
        messages.push(answered.clone());
        messages.push(json!({ "role": "user", "content": "thanks" }));
        rewrite_messages(&mut messages, true);
        assert_eq!(messages[1], waiting);
        assert_eq!(messages[3], answered);
    }

    #[test]
    fn active_replay_drops_a_search_left_without_result() {
        // A paused turn ended on a search that never ran.
        let mut messages = vec![
            json!({ "role": "user", "content": "q" }),
            json!({ "role": "assistant", "content": [
                { "type": "text", "text": "Searching." }, call("srv_1") ] }),
            json!({ "role": "user", "content": "go on" }),
        ];
        rewrite_messages(&mut messages, true);
        assert_eq!(
            messages[1]["content"],
            json!([{ "type": "text", "text": "Searching." }])
        );
    }

    #[test]
    fn inactive_replay_strips_search_blocks_and_their_citations() {
        let mut messages = vec![
            json!({ "role": "user", "content": "q" }),
            json!({ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "t", "signature": "s" },
                call("srv_1"), result("srv_1"), cited_text(),
                { "type": "text", "text": "plain",
                  "citations": [{ "type": "char_location", "cited_text": "x" }] } ] }),
            json!({ "role": "user", "content": "q2" }),
            json!({ "role": "assistant", "content": [ call("srv_2"), result("srv_2") ] }),
            json!({ "role": "user", "content": "q3" }),
        ];
        rewrite_messages(&mut messages, false);
        assert_eq!(
            messages[1]["content"],
            json!([
                { "type": "thinking", "thinking": "t", "signature": "s" },
                { "type": "text", "text": "Rust 1.95 is out." },
                { "type": "text", "text": "plain",
                  "citations": [{ "type": "char_location", "cited_text": "x" }] }
            ])
        );
        // A message of searches only is removed, not sent empty.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[3], json!({ "role": "user", "content": "q3" }));
    }

    #[test]
    fn stream_blocks_become_hosted_calls() {
        let started = hosted_call_at_start(
            &json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search" }),
            |_| None,
        );
        let Some(LlmStreamEvent::HostedToolCall(started)) = started else {
            panic!("expected a hosted call");
        };
        assert_eq!(started.id, "srv_1");
        assert_eq!(started.tool, "web_search");
        assert_eq!(started.status, HostedToolCallStatus::InProgress);
        assert_eq!(started.summary, None);

        let mut streamed = json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search",
                                   "input": { "query": "rust release" } });
        let Some(LlmStreamEvent::HostedToolCall(queried)) = hosted_call_at_stop(&mut streamed)
        else {
            panic!("expected a hosted call");
        };
        assert_eq!(queried.summary.as_deref(), Some("rust release"));

        let mut bare = json!({ "type": "server_tool_use", "id": "srv_2", "name": "web_search" });
        assert!(hosted_call_at_stop(&mut bare).is_none());
        assert_eq!(bare["input"], json!({}));

        let Some(LlmStreamEvent::HostedToolCall(done)) =
            hosted_call_at_start(&result("srv_1"), |id| {
                (id == "srv_1").then(|| "rust release".to_string())
            })
        else {
            panic!("expected a hosted call");
        };
        assert_eq!(done.status, HostedToolCallStatus::Completed);
        assert_eq!(done.summary.as_deref(), Some("rust release"));

        let error = json!({ "type": "web_search_tool_result", "tool_use_id": "srv_3",
            "content": { "type": "web_search_tool_result_error", "error_code": "max_uses_exceeded" } });
        let Some(LlmStreamEvent::HostedToolCall(failed)) = hosted_call_at_start(&error, |_| None)
        else {
            panic!("expected a hosted call");
        };
        assert_eq!(failed.status, HostedToolCallStatus::Failed);

        // Tool search's server calls are not web searches.
        let tool_search = json!({ "type": "server_tool_use", "id": "srv_4",
                                  "name": "tool_search_tool_bm25", "input": {} });
        assert!(hosted_call_at_start(&tool_search, |_| None).is_none());
    }

    #[test]
    fn citations_span_their_text_block_in_chars() {
        let content = vec![
            json!({ "type": "text", "text": "Zürich 😀: " }),
            call("srv_1"),
            result("srv_1"),
            cited_text(),
            json!({ "type": "text", "text": " Über.", "citations": [
                { "type": "char_location", "cited_text": "x" },
                { "type": "web_search_result_location", "url": "https://ü.de",
                  "encrypted_index": "E", "cited_text": "über" } ] }),
        ];
        let cites = citations(&content);
        assert_eq!(cites.len(), 2);
        let streamed = "Zürich 😀: Rust 1.95 is out. Über.";
        let first = cites[0].span.as_ref().unwrap();
        assert_eq!((first.start, first.end), (10, 27));
        assert_eq!(first.text, "Rust 1.95 is out.");
        let covered: String = streamed.chars().skip(first.start).take(17).collect();
        assert_eq!(covered, first.text);
        assert_eq!(cites[0].url, "https://blog.rust-lang.org");
        assert_eq!(cites[0].title.as_deref(), Some("Rust"));
        assert_eq!(cites[0].snippet.as_deref(), Some("1.95"));
        let second = cites[1].span.as_ref().unwrap();
        assert_eq!((second.start, second.end), (27, 33));
        assert_eq!(cites[1].title, None);
        assert!(citations(&[call("srv_1"), result("srv_1")]).is_empty());
    }

    #[test]
    fn searches_count_from_usage_or_results() {
        let error = json!({ "type": "web_search_tool_result", "tool_use_id": "srv_3",
            "content": { "type": "web_search_tool_result_error", "error_code": "unavailable" } });
        let content = vec![call("srv_1"), result("srv_1"), error];
        assert_eq!(search_count(0, &content), 1);
        assert_eq!(search_count(2, &content), 2);
        assert_eq!(
            reported_searches(Some(&json!({ "web_search_requests": 3 }))),
            Some(3)
        );
        assert_eq!(reported_searches(None), None);
    }
}
