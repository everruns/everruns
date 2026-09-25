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
