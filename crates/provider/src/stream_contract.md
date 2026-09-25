# Stream contract

What every driver guarantees, so a host can consume a stream without
reading provider source.

**Order.** A stream is zero or more content events, then exactly one
terminal event: [`Done`](Self::Done) on success or [`Error`](Self::Error)
on failure. Nothing follows a terminal event. A stream that ends without
one was cut off; [`crate::turn_collector`] reports that as incomplete when
the call's limits require a terminal event.

**Deltas are live, items are the record.** [`TextDelta`](Self::TextDelta)
and [`ReasoningDelta`](Self::ReasoningDelta) are for display as they
arrive. [`ReasoningItem`](Self::ReasoningItem) and
[`ToolCalls`](Self::ToolCalls) are the persistable artifacts, emitted once
a block completes (for Chat Completions, just before `Done`). A
`ReasoningItem` repeats the full text its deltas already streamed, plus
any opaque replay state, so a consumer either renders deltas and stores
items, or ignores deltas and uses items alone. Concatenating both
duplicates the reasoning.

**Assistant text** is the concatenation of `TextDelta`s; there is no
separate text item. Through [`Provider`](crate::runtime_provider::Provider)
every `TextDelta` carries content. A driver called directly may still use
an empty `TextDelta` as filler for wire frames with nothing to report
(usage, keep-alive); skip those.

**Tool calls** arrive whole, arguments already parsed, in one or more
`ToolCalls` events. Arguments that were not valid JSON degrade to `{}` when
the provider explicitly finished the call, and drop the call otherwise;
either case logs a `tracing` warning.

**`Done`** carries [`LlmCompletionMetadata`]: usage, the served model, and
`finish_reason` as the provider reported it, normalized to the Chat
Completions vocabulary (`stop`, `length`, `tool_calls`, `content_filter`,
...). `finish_reason` is `None` when the provider sent no reason; drivers
do not substitute `stop`, so a host can fail closed on a missing reason.

**`Error`** carries an [`LlmStreamError`] that keeps the provider's
message, error code and HTTP status. That includes error envelopes a
gateway sends inside a `200` stream.

**Retries happen before the first event.** HTTP drivers retry `429` and
transient `5xx` responses while establishing the stream, under
[`LlmRetryConfig`](crate::llm_retry::LlmRetryConfig): by default up to 2
retries with exponential backoff and at most 30 seconds of recovery. That
time counts against any timeout the host wraps around the call, so a host
that wants to own retries sets
[`LlmRetryConfig::no_retry`](crate::llm_retry::LlmRetryConfig::no_retry)
on the driver. `Done` reports what happened in
[`retry_metadata`](LlmCompletionMetadata::retry_metadata).

# Example: consuming a stream

A consumer that renders text and reasoning live, keeps the reasoning and
tool-call items as the record, and treats a missing finish reason as a
failure. The stream here is scripted; a real one comes from
[`Provider::chat_completion_stream`](crate::runtime_provider::Provider::chat_completion_stream).

```
use everruns_provider::driver_registry::{
    LlmCompletionMetadata, LlmResponseStream, LlmStreamError, LlmStreamEvent,
};
use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_provider::tool_types::ToolCall;
use futures::StreamExt;

#[derive(Default)]
struct Turn {
    text: String,
    reasoning: Vec<ReasoningContentPart>,
    tool_calls: Vec<ToolCall>,
}

async fn consume(mut stream: LlmResponseStream) -> Result<Turn, String> {
    let mut turn = Turn::default();
    while let Some(event) = stream.next().await {
        match event.map_err(|error| error.to_string())? {
            // Live: show it, and build the text from it.
            LlmStreamEvent::TextDelta(delta) => turn.text.push_str(&delta),
            // Live: show it, but do not store it. The item below repeats it.
            LlmStreamEvent::ReasoningDelta { delta, .. } => print!("{delta}"),
            // The record: store these.
            LlmStreamEvent::ReasoningItem(item) => turn.reasoning.push(item),
            LlmStreamEvent::ToolCalls(calls) => turn.tool_calls.extend(calls),
            LlmStreamEvent::Done(metadata) => {
                // `None` means the provider never said how the turn ended.
                return match metadata.finish_reason.as_deref() {
                    Some("stop" | "tool_calls") => Ok(turn),
                    Some(other) => Err(format!("turn ended with {other}")),
                    None => Err("no finish reason: stream may be truncated".into()),
                };
            }
            // Status and code are the provider's own, not parsed from text.
            LlmStreamEvent::Error(error) => {
                return Err(format!("{:?} {:?}: {}", error.status, error.code, error.message));
            }
            // `#[non_exhaustive]`: skip event kinds this code does not know.
            _ => {}
        }
    }
    Err("stream ended without Done or Error".into())
}

fn scripted(events: Vec<LlmStreamEvent>) -> LlmResponseStream {
    Box::pin(futures::stream::iter(events.into_iter().map(Ok)))
}

fn done(finish_reason: Option<&str>) -> LlmStreamEvent {
    let mut metadata = LlmCompletionMetadata::default();
    metadata.finish_reason = finish_reason.map(str::to_owned);
    LlmStreamEvent::Done(Box::new(metadata))
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() {
let thought = "Plan: greet.";
let turn = consume(scripted(vec![
    LlmStreamEvent::ReasoningDelta { delta: thought.into(), summary: false },
    LlmStreamEvent::TextDelta("Hello".into()),
    LlmStreamEvent::ReasoningItem(
        ReasoningContentPart::opaque("example")
            .with_text(ReasoningText::Plain { text: thought.into() }),
    ),
    done(Some("stop")),
]))
.await
.unwrap();
assert_eq!(turn.text, "Hello");
// Stored once, from the item, not once per delta plus once more.
assert_eq!(turn.reasoning.len(), 1);

// A stream whose provider never sent a finish reason.
let missing = consume(scripted(vec![LlmStreamEvent::TextDelta("Hel".into()), done(None)])).await;
assert!(missing.is_err());

// A gateway error inside a `200` stream keeps its status and message.
let failed = consume(scripted(vec![LlmStreamEvent::Error(LlmStreamError::provider(
    None::<String>,
    Some(502),
    "upstream died",
))]))
.await;
assert_eq!(failed.err().unwrap(), "Some(502) None: upstream died");
# }
```

# Example: owning retries in the host

A host that already runs its own retry loop, or wraps calls in a tight
timeout, turns driver retries off so a `429` reaches it at once:

```
use everruns_provider::{BearerAuth, LlmRetryConfig, OpenAIProtocolChatDriver, Provider};

let provider = Provider::new(
    "gateway",
    OpenAIProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry()),
)
.base_url("http://127.0.0.1:8081/v1")
.auth(BearerAuth::new("local-key"));
# let _ = provider;
```

The vendor drivers (`everruns-openai`, `everruns-openrouter`,
`everruns-gemini`, `everruns-anthropic`) take the same `with_retry_config`.
