---
title: Prompt Caching
description: Turns on provider prompt caching for Anthropic, OpenAI, and Gemini requests, and records the cache mode on each llm.generation event.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `prompt_caching` |
| **Category** | Optimization |
| **Tools** | None |
| **Features** | None |
| **Dependencies** | None |

Prompt caching lets a provider reuse the unchanged start of a request (system
prompt, tool definitions, earlier conversation) instead of processing it again
on every model call. Cached input is usually cheaper and faster. This capability
sets the request options each driver needs. It adds no tools and no prompt
text.

Drivers that support caching translate the setting into their provider's
request fields. A driver or model without support ignores it, and the request
proceeds unchanged.

The [Platform Chat](/built-ins/harnesses/platform-chat/) harness includes it.

## What each provider gets

| Provider | With `strategy: auto` (default) | With `strategy: explicit` |
|---|---|---|
| Anthropic | `cache_control` breakpoints on the system prompt, the last tool definition, and the two most recent stable conversation messages, four in total | Breakpoints on the system prompt and the last tool definition only, so the conversation is never written to cache |
| OpenAI | A `prompt_cache_key` derived from the session (or agent, harness, or organization), model, instructions, and tools. On models whose profile supports cache options, also `prompt_cache_options` with mode `implicit` and a 30 minute TTL | `prompt_cache_key`, and on supporting models `prompt_cache_options` with mode `explicit`: the instructions move into a developer message marked as the cache breakpoint, so only that prefix is written to cache |
| Gemini | The provider's default behavior, such as implicit caching on supported models | Same as `auto` |

On Gemini, setting `gemini_cached_content` sends that existing cached content
resource in the request's `cachedContent` field.

On OpenAI with the explicit mode, the driver replays the full transcript
instead of chaining with `previous_response_id`, so the developer prefix is not
appended again on each continuation.

Use `explicit` when many short, unrelated sessions share long instructions or
tools, such as a support agent: caching every conversation would cost more in
cache writes than it saves in reads.

## Retention

`retention` sets how long the provider keeps a cached prefix after its last
use. Each driver rounds it to the nearest value its provider supports.

| Provider | `default`, `5m` | `30m` | `1h` |
|---|---|---|---|
| Anthropic | 5 minutes | 5 minutes | 1 hour (`ttl: "1h"` on every breakpoint) |
| OpenAI with cache options | 30 minutes | 30 minutes | 30 minutes |
| Others | Provider default | Provider default | Provider default |

A longer retention costs more per write: Anthropic bills a 1 hour cache write at
2x the input rate instead of 1.25x, and cost estimates include that. Choose `1h`
for agents whose turns are often more than five minutes apart, such as agents
that wait on a person or run on a schedule, where a 5 minute cache would have
expired before the next turn.

## Configuration

```json
{
  "ref": "prompt_caching",
  "config": { "strategy": "auto", "retention": "1h" }
}
```

| Field | Default | Description |
|---|---|---|
| `strategy` | `auto` | `auto` lets each driver pick its provider behavior. `explicit` caches only the stable prefix (instructions, and tools on Anthropic) on providers that support it. |
| `retention` | `default` | `default`, `5m`, `30m`, or `1h`. Rounded to what the provider supports. |
| `gemini_cached_content` | none | An existing Gemini cached content resource name, `cachedContents/{id}` |

## Observability

Each `llm.generation` event records the request in
`request_options.prompt_cache`: whether caching was enabled, the strategy, and
a `provider_mode` naming what the driver did, and the `retention` it requested
after rounding. The modes are `cache_control`
(Anthropic), `implicit`, `explicit`, or `prompt_cache_key` (OpenAI), and
`implicit` or `cached_content` (Gemini). Cache read and write token counts, when
the provider reports them, appear in the event's token usage.

In a session's Trace view, each model call shows its prompt split into cache
read, cache write and uncached tokens. When a call reuses less than half of
what the previous call left cached, the trace marks a cache reset and names the
likely cause: the model, the tool definitions, the system prompt or earlier
messages changed, or the session sat idle longer than the cache retention. A
long prompt sent with caching off is marked as one that caching would help.

## See also

- [Platform Chat harness](/built-ins/harnesses/platform-chat/)
- [OpenTelemetry](/observability/opentelemetry/), which exports prompt-cache token counts per call
- [Tool Search](/capabilities/tool-search/), another request-level optimization
