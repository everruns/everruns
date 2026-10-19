---
title: Google Gemini
description: Run Everruns agents on Google Gemini models, with streaming, tool calls, reasoning, context caching, and long decode on Gemini 4 Argon.
sidebar:
  label: Google Gemini
---

<svg role="img" aria-label="Google Gemini logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81"/></svg>

Everruns runs agents on [Google Gemini](https://ai.google.dev/) models,
implementing the provider-neutral driver contract over the Gemini API.

## What you get

- **Gemini API** streaming.
- **Gemini 4 Argon long decode**: one response can continue across requests with
  a shared output budget.
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

Gemini 4 Argon is seeded as an enabled favourite. New Gemini setups prefer it
when choosing an organisation default; existing defaults stay selected.

Gemini models resolve to Everruns' built-in model profiles for capability and
cost metadata. Argon defaults to 65,536 output tokens. Other Gemini models
resolve their default from the model profile, falling back to a safe value.

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

With native Google Search enabled, citations retain the sources and supported
answer spans from every hop, including when the final hop returns no grounding.
Repeated search metadata within a hop is counted once; the same query in another
hop counts as a separate search. Gemini 3 and later count queries for search
usage, while Gemini 2.x counts grounded requests.

Argon supports low, medium, and high thinking levels; minimal maps to low and
higher efforts map to high. Sampling temperature is omitted. Argon supports
implicit caching only; do not supply an explicit cached-content resource.

Long-decode calls default to a 20-minute HTTP read and agent-loop stall timeout.
Explicit host-level turn and stall limits still apply. A failed hop
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
