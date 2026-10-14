---
title: Google Gemini
description: Run Everruns agents on Google Gemini models, with streaming, tool calls, reasoning, and context caching.
sidebar:
  label: Google Gemini
---

<svg role="img" aria-label="Google Gemini logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M12 0C12 6.627 6.627 12 0 12c6.627 0 12 5.373 12 12 0-6.627 5.373-12 12-12-6.627 0-12-5.373-12-12z"/></svg>

Everruns runs agents on [Google Gemini](https://ai.google.dev/) models,
implementing the provider-neutral driver contract over the Gemini API.

## What you get

- **Gemini API** streaming.
- **Tool calls and reasoning** mapped to provider-neutral Everruns types.
- **Context caching**: explicit caching via `cachedContent` when a cached-content
  resource is supplied, otherwise Gemini's default implicit caching on supported
  models.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Google Gemini**.
3. Paste your Gemini API key. Get one from
   [Google AI Studio](https://aistudio.google.com/apikey).
4. Save. Everruns discovers available Gemini models automatically.

## Models

Gemini models resolve to Everruns' built-in model profiles for capability and
cost metadata. When `max_tokens` is not set, the driver resolves a default from
the model profile, falling back to a safe value.

## Long decode

For `gemini-4-argon`, set `max_tokens` above 65,536 to let a single response
continue across multiple Gemini API requests. In an agent package, use
the top-level `max_output_tokens`; the built-in Argon profile permits up to 1,048,576
tokens. Input, thinking, and output share that window, so leave room for the
prompt. The default remains 65,536.

```rust
use everruns_contracts::{LlmCallConfig, ReasoningEffort};

let mut config = LlmCallConfig::new("gemini-4-argon");
config.max_tokens = Some(100_000);
config.reasoning_effort = Some(ReasoningEffort::High);
```

Everruns follows continuation tokens automatically, streams text and reasoning
across the hops, and returns one completion. Usage sums the final counts from
each hop, including billed thinking and repeated input. Context-window pressure
is estimated from the assembled messages, rather than those summed billing counts.

Argon supports low, medium, and high thinking levels; minimal maps to low and
higher efforts map to high. Sampling temperature is omitted. Argon supports
implicit caching only; do not supply an explicit cached-content resource.

Each Argon HTTP stream permits a 20-minute read timeout. Host-level turn and
stall limits still apply; configure them for long reasoning tasks. A failed hop
fails the call. A dropped stream resumes only when no output was exposed after
its last checkpoint; otherwise it fails to prevent duplicate output. Cancelling
or dropping the stream stops local continuation requests.

See Google's [long decode guide](https://ai.google.dev/gemini-api/docs/generate-content/long-decode)
for provider limits and billing.

## Links

- [Google AI for Developers](https://ai.google.dev/)
- [Google AI Studio](https://aistudio.google.com/)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers), feature `gemini`
- [Migrate between providers](/how-to/migrate-providers/)
