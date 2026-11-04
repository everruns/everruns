---
title: Mistral AI
description: Run Everruns agents on Mistral models (Mistral Large 4, Medium, Small, Codestral) through La Plateforme, with reasoning and automatic model discovery.
sidebar:
  label: Mistral AI
---

<svg role="img" aria-label="Mistral AI logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M17.143 3.429v3.428h-3.429v3.429h-3.428V6.857H6.857V3.43H3.43v13.714H0v3.428h10.286v-3.428H6.857v-3.429h3.429v3.429h3.429v-3.429h3.428v3.429h-3.428v3.428H24v-3.428h-3.43V3.429z"/></svg>

Everruns runs agents on [Mistral AI](https://mistral.ai/) models through La
Plateforme's Chat Completions API: Mistral Large 4, Mistral Medium, Mistral
Small, Ministral and Codestral, with the same agent, prompt, and capabilities
you run on any other provider.

## What you get

- **Mistral Large 4**: Mistral's 1T-parameter multimodal model with a 512K
  context window, tool calling, image input, structured output, and reasoning.
- **Reasoning on demand**: Mistral Large 4's thinking is on or off. Set the
  agent's reasoning effort to **On** (`high`) to stream its reasoning
  separately from the answer; the default, **Off**, answers directly.
- **Automatic model discovery**: Mistral's `/models` endpoint advertises chat,
  tool calling, vision and reasoning support per model, which Everruns turns
  into capability profiles on sync. Aliases are folded into one entry per
  model, and models Mistral has scheduled for retirement are skipped.
- **Host-gated discovery**: model sync runs only against `api.mistral.ai`, so a
  custom proxy base URL is never probed.

## Configure in Everruns

The Mistral AI provider is behind the `mistral` feature flag, which is
off by default. A self-hosted deployment turns it on by setting
`FEATURE_MISTRAL=prod` (on for every organization) or `adoption`
(organizations opt in). Until then, Mistral AI does not appear in the provider
picker and the API refuses to create a `mistral` provider. Mistral models remain
reachable through OpenRouter either way.

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Mistral AI**.
3. Paste your API key. Create one in
   [Mistral Studio](https://console.mistral.ai/api-keys).
4. Save. Everruns discovers the available models and their capabilities.

You can optionally set a base URL to route through a proxy; leave it blank to
use `https://api.mistral.ai/v1`.

## Models

Use Mistral's model ids, for example `mistral-large-4` (alias
`mistral-large-4-0`), `mistral-medium-latest`, or `codestral-latest`.
Mistral Large 4 is also reachable through [OpenRouter](/providers/openrouter/)
as `mistralai/mistral-large-4-0`, with the same profile.

Everruns ships profiles (context, pricing, reasoning toggle) for Mistral
Large 4, Mistral Medium 3.5 (`mistral-medium-2604`, also
`mistral-medium-latest`), and Mistral Small 4 (`mistral-small-2603`, also
`mistral-small-latest`).

## Framework

```rust
// Reads MISTRAL_API_KEY, and MISTRAL_BASE_URL when set.
let provider = everruns::Mistral::from_env()?;
```

Enable the `mistral` feature of `everruns`.

## Responses API preview

The existing Mistral provider can opt into Mistral's stateless
[Responses API](https://docs.mistral.ai/studio/conversations/responses) per connection.
Chat Completions remains the default. Select Responses when constructing the
provider connection (enable the `mistral` feature of `everruns`):

```rust
use everruns::{Agent, Mistral, MistralApi};

let agent = Agent::builder()
    .provider(Mistral::from_env()?.api(MistralApi::Responses))
    .model("mistral-large-4")
    .instructions("Be concise.")
    .build()?;
```

For a direct call, convert the same configuration into `everruns::Provider`:

```rust
use everruns::{Mistral, MistralApi, Provider};

let provider: Provider = Mistral::from_env()?
    .api(MistralApi::Responses)
    .into();
```

Use the same API key and `/v1` base URL. `MistralApi::ChatCompletions` selects
`/v1/chat/completions`; `MistralApi::Responses` selects `/v1/responses`.
Shared call configuration carries the model and generation settings, not the protocol.
The runnable `mistral_responses` example uses this configuration:
`cargo run -p everruns --features mistral --example mistral_responses`.
Low-level hosts can use `everruns_drivers::mistral::provider_with_api(id, key, api)`.

Hosted REST clients select the same API when creating or updating a provider:

```json
{
  "name": "Mistral Responses",
  "provider_type": "mistral",
  "api": "responses"
}
```

Configure credentials through the normal provider credential fields. `api` may be
`responses`, `chat_completions`, or `default`. Omitting it on create preserves the
existing default; omitting it on PATCH preserves the stored selection. PATCH
`"api": "default"` restores the default. The server validates choices against the
driver catalog. API reads return the stored selection, and `/v1/providers/config`
returns supported choices in each driver's `apis` field. Server and worker must
both run a version supporting this field. The Settings UI has no selector yet.

Responses supports streamed text and reasoning, client-executed function calls,
parallel tool calls, JSON Schema output, token usage, prompt-cache routing, and
HTTP retries. Pass the full message history on every call: Everruns replays
messages, function calls and results, and Mistral plaintext reasoning. It sends
`store: false` and ignores `previous_response_id` because Mistral stores no
response chain. Reasoning from another provider is not replayed to Mistral.

User messages and function results can contain image URLs/data URLs and files
as HTTPS URLs or data URLs, subject to the selected model's capabilities.
Uploaded file IDs are not exposed by Everruns' portable message contract.
Audio and opaque provider content are rejected before sending a request.

Non-streaming callers receive a collected SSE response; native Responses JSON
transport is not implemented. Failed, incomplete, or prematurely closed streams
fail the call rather than reporting a successful partial answer. Background
responses, WebSockets, server-side compaction, hosted tools, token-counting,
function namespaces, and forced tool selection are not exposed by this
integration. OpenAI-only service-tier and verbosity settings are omitted.
The existing Mistral feature flag and model discovery behavior are unchanged.

## Links

- [Mistral Large 4 announcement](https://mistral.ai/news/mistral-large-4/)
- [Mistral API docs](https://docs.mistral.ai/)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers), feature `mistral`
- [Migrate between providers](/how-to/migrate-providers/)

## Native Framework batches

Enable the `mistral` Cargo feature of `everruns` for direct offline model
requests through [Batch Model Requests](/framework/batch-model-requests/). Mistral uses native inline jobs with resumable tickets and explicit cancellation.

The API is on `main`; published `0.49.0` does not include it. Use the guide’s Git dependency until a release includes this API.
