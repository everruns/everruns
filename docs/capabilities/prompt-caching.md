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
| Anthropic | `cache_control` breakpoints on the system prompt, the last tool definition, and the two most recent stable conversation messages, four in total | Same as `auto` |
| OpenAI | A `prompt_cache_key` derived from the session (or agent, harness, or organization), model, instructions, and tools. On models whose profile supports cache options, also `prompt_cache_options` with mode `implicit` and a 30 minute TTL | `prompt_cache_key`, and on supporting models `prompt_cache_options` with mode `explicit`: the instructions move into a developer message marked as the cache breakpoint, so only that prefix is written to cache |
| Gemini | The provider's default behavior, such as implicit caching on supported models | Same as `auto` |

On Gemini, setting `gemini_cached_content` sends that existing cached content
resource in the request's `cachedContent` field.

On OpenAI with the explicit mode, the driver replays the full transcript
instead of chaining with `previous_response_id`, so the developer prefix is not
appended again on each continuation.

## Configuration

```json
{
  "ref": "prompt_caching",
  "config": { "strategy": "auto" }
}
```

| Field | Default | Description |
|---|---|---|
| `strategy` | `auto` | `auto` lets each driver pick its provider behavior. `explicit` caches only the instruction prefix on models that support it. |
| `gemini_cached_content` | none | An existing Gemini cached content resource name, `cachedContents/{id}` |

## Observability

Each `llm.generation` event records the request in
`request_options.prompt_cache`: whether caching was enabled, the strategy, and
a `provider_mode` naming what the driver did. The modes are `cache_control`
(Anthropic), `implicit`, `explicit`, or `prompt_cache_key` (OpenAI), and
`implicit` or `cached_content` (Gemini). Cache read and write token counts, when
the provider reports them, appear in the event's token usage.

## See also

- [Platform Chat harness](/built-ins/harnesses/platform-chat/)
- [OpenTelemetry](/observability/opentelemetry/), which exports prompt-cache token counts per call
- [Tool Search](/capabilities/tool-search/), another request-level optimization
