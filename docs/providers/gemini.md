---
title: Google Gemini
description: Run Everruns agents on Google Gemini models, including the Gemini 4 family, with streaming, tool calls, reasoning, context caching, and long decode.
sidebar:
  label: Google Gemini
---

<svg role="img" aria-label="Google Gemini logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81"/></svg>

Everruns runs agents on [Google Gemini](https://ai.google.dev/) models,
implementing the provider-neutral driver contract over the Gemini API.

## What you get

- **Interactions API** streaming, with the conversation kept on Google's side
  between turns (see [Server-kept conversation](#server-kept-conversation)).
- **Gemini 4 long decode** (Hydrogen, Neon, Carbon, Argon): one response can
  continue across requests with a shared output budget.
- **Tool calls and reasoning** mapped to provider-neutral Everruns types.
- **Native computer use** on Gemini 4: the
  [computer use](/capabilities/computer-use/) capability hands Gemini its
  built-in `computer_use` tool on Google's own endpoint.
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

| Model | Best for | Thinking levels | Price per 1M tokens (input / output / cached) |
|---|---|---|---|
| `gemini-4-hydrogen` | Everyday agent work; fastest and cheapest | low, medium, high, xhigh | $1.50 / $7.50 / $0.075 |
| `gemini-4-neon` | Complex software engineering | low, medium, high, xhigh | $3.00 / $15.00 / $0.15 |
| `gemini-4-carbon` | Hardest reasoning and research | low, medium, high | $2.00 / $10.00 / $0.10 until 30 November 2026, then $4 / $20 / $0.20 |
| `gemini-4-argon` | The first Gemini 4 model | low, medium, high | $2.00 / $10.00 / $0.10 until 30 November 2026, then $4 / $20 / $0.20 |

Gemini 4 Hydrogen, Neon, Carbon, and Argon are seeded and enabled; Hydrogen and
Neon are favourites. New Gemini setups prefer Hydrogen when choosing an
organisation default; existing defaults stay selected. The provider recommends
Neon for coding agents and Carbon for research.

Gemini models resolve to Everruns' built-in model profiles for capability and
cost metadata. Gemini 4 models default to 65,536 output tokens. Other Gemini
models resolve their default from the model profile, falling back to a safe value.

## Server-kept conversation

On Google's own endpoint, agent turns use Gemini's
[Interactions API](https://ai.google.dev/gemini-api/docs/interactions), the
interface Google recommends since June 2026. Google stores each interaction
and Everruns keeps its id. The next model call sends that id and only what
happened since the model last answered, such as tool results and new
messages. The earlier transcript and its thought signatures are not sent
again, so long tool loops send much less per call and Google's implicit
cache covers the stored prefix.

The system prompt, tools and thinking settings travel with every call, so a
change to the agent applies on the next call. When the stored interaction is
gone (Google keeps it 55 days on paid projects and one day on the free tier,
and you can delete it in AI Studio's Logs), Everruns sends the whole
transcript once and continues from the new interaction.

Stored interactions appear in AI Studio's Logs for the API key's project.
Calls that need a feature Interactions does not offer yet use
`generateContent` and resend the transcript as before: batch requests, an
explicit cached-content resource, long decode above 65,536 output tokens, and
providers configured with a custom base URL.

## Long decode

For any Gemini 4 model, set `max_tokens` above 65,536 to let a single response
continue across multiple Gemini API requests. In an agent package, use
the top-level `max_output_tokens`; the built-in Gemini 4 profiles permit up to 1,048,576
tokens. Input, thinking, and output share that window, so leave room for the
prompt. The default remains 65,536.

```rust
use everruns_contracts::{LlmCallConfig, ReasoningEffort};

let mut config = LlmCallConfig::new("gemini-4-neon");
config.max_tokens = Some(100_000);
config.reasoning_effort = Some(ReasoningEffort::High);
```

Everruns follows continuation tokens automatically, streams text and reasoning
across the hops, and returns one completion. Usage sums the final counts from
each hop, including billed thinking and repeated input. Tool-use prompt tokens
reported for provider-hosted tools count as additional uncached input; cached
tokens remain a subset of the ordinary prompt. Repeated cumulative usage frames
within a hop are counted once. Context-window pressure
is estimated from the assembled messages, rather than those summed billing counts.

With native Google Search enabled, citations retain the sources and supported
answer spans from every hop, including when the final hop returns no grounding.
Repeated search metadata within a hop is counted once; the same query in another
hop counts as a separate search. Gemini 3 and later count queries for search
usage, while Gemini 2.x counts grounded requests.

Gemini 4 models take thinking levels rather than budgets; minimal maps to low.
Hydrogen and Neon accept `xhigh`, so Extra High and Max map to it there; Carbon
and Argon map every effort above medium to high. Sampling temperature is omitted.
Gemini 4 models support implicit caching only; do not supply an explicit
cached-content resource.

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
