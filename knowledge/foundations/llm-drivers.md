---
type: Specification
title: "LLM Drivers Specification"
description: "LLM driver trait, provider implementations."
tags:
  - everruns
  - foundations
---
# LLM Drivers Specification

> **Domain model note:** the canonical provider domain model (drivers, services,
> providers, models, model profiles) is [providers.md](providers.md). That
> `ChatDriver` is the chat wire-protocol contract. Service identity, endpoint,
> headers, and authentication belong to runtime providers as specified in
> [providers.md](providers.md).

## Abstract

LLM drivers implement reusable wire protocols. Runtime providers bind those
drivers to concrete services. Core business logic resolves a credential-free
model specification and runtime provider independently, then passes the
provider-owned endpoint into the protocol driver.

Official driver packages are physically grouped under
[`crates/drivers/`](../../crates/drivers/README.md). The folder is only a
repository organization boundary: every child remains an independently
versioned crate, and `everruns-provider` remains the neutral SPI they
implement.

## Architecture

```mermaid
graph TD
    subgraph Provider [everruns-provider]
        ChatDriver[ChatDriver Trait]
        Registry[ProviderRegistry]
        Errors[AgentLoopError]
    end

    subgraph Providers
        OpenAI[everruns-openai]
        Anthropic[everruns-anthropic]
        Gemini[everruns-gemini]
    end

    subgraph Host
        Egress[EgressService]
    end

    subgraph Consumer
        ReasonAtom[ReasonAtom]
    end

    OpenAI -->|assembles provider over| ChatDriver
    Anthropic -->|assembles provider over| ChatDriver
    Gemini -->|assembles provider over| ChatDriver
    OpenAI -->|registers provider| Registry
    Anthropic -->|registers provider| Registry
    Gemini -->|registers provider| Registry
    ReasonAtom -->|resolves provider from| Registry
    OpenAI -->|outbound HTTP| Egress
    Anthropic -->|outbound HTTP| Egress
    Gemini -->|outbound HTTP| Egress
    ReasonAtom -->|handles| Errors
```

## Requirements

### ChatDriver Trait

1. **Trait Definition**: See `crates/provider/src/driver_registry.rs` for `ChatDriver` trait, `ProviderType`, and `LlmCallConfig`, and `crates/provider/src/stream_event.rs` for `LlmStreamEvent`.

2. **Streaming Response**: Drivers return a stream of `LlmStreamEvent` (TextDelta, ToolCalls, ThinkingDelta, ThinkingSignature, Done, Error). In-band provider failures use `LlmStreamError` so stable provider code and HTTP status survive the driver boundary.

3. **Provider independence**: the trait contains no service/vendor identity,
   default hostname, or credential. The same driver instance may serve multiple
   provider keys with different endpoints, headers, and auth.

4. **`ToolCalls` carries the whole set, and a driver owes it before `Done`.**
   The stream reader *replaces* its tool-call list on every `ToolCalls` event,
   so an event carrying only the newest call drops the earlier ones; and `Done`
   ends the stream for the reader, so a call surfaced after it never runs. A
   driver whose wire protocol describes one call across several frames must
   therefore treat every frame that names it as authoritative, not just the
   first, and reconcile against the provider's own terminal call list before
   emitting `Done`. Nothing downstream can audit this: the finish reason a
   driver reports is derived from what it emitted, so a dropped call is
   indistinguishable from the model choosing to stop.

### Error Types (Contract)

Drivers MUST use the following error types from `AgentLoopError`:

1. **`Llm(String)`** - Generic LLM provider error
   - Use for: network errors, authentication failures, invalid requests, server errors
   - Example: `AgentLoopError::llm("OpenAI API error (500): Internal server error")`

2. **`RequestTooLarge(String)`** - Context length or token limit exceeded
   - Use for: context length exceeded, token limits hit, prompt too long
   - Drivers MUST detect provider-specific error responses and convert to this type
   - Example: `AgentLoopError::request_too_large("OpenAI API error (429): Request too large...")`

3. **`Llm` with transport detail preserved** - An HTTP driver MUST build its
   terminal failure with `AgentLoopError::llm_http` (or `llm_http_kind` when
   the driver's protocol extension classifies more precisely). These classify
   *and* record the HTTP status, the provider's own error code, and any
   requested retry delay on `LlmError`, at the boundary where the response is
   still structured. Anything downstream that re-expresses the failure — an
   HTTP API in front of Everruns, a retry budget of its own — then reads those
   fields instead of scraping the display string, which is not a contract.
   `AgentLoopError::http_status()` also answers for the semantic variants
   (`ModelNotAvailable` is 404, `RequestTooLarge` is 413).

### Error Detection Requirements

Each driver MUST implement provider-specific error detection to classify context-length and token-limit errors as `RequestTooLarge`. A per-window quota or rate-limit rejection (a 429 whose request would fit once the window resets) is transient and MUST stay on the retry path, even when its wording mentions tokens and limits; only a request that exceeds the limit on its own is `RequestTooLarge`. See the individual driver crates for the detection logic:
- `crates/provider/src/openai_errors.rs`, shared OpenAI-compatible (Chat Completions and Responses) detection
- `crates/drivers/openai/src/`, OpenAI error detection
- `crates/drivers/anthropic/src/`, Anthropic error detection
- `crates/drivers/gemini/src/`, Gemini error detection

### Provider registry and 0.17 compatibility catalog

Applications register concrete runtime providers in `ProviderRegistry`, keyed
by open `ProviderKey`. Official integration crates expose ready-made provider
assemblies; downstream code may construct the same public `Provider` directly.

`DriverDescriptor`/`DriverRegistry` remain as the hosted provider-management
catalog and the host-facing surface for `everruns-host`. Descriptor factories
compose runtime providers and route into the same registry/execution path; they
are not a second driver semantics layer.

### Host-Owned Transport

Protocol drivers and model discovery use direct provider HTTP clients. They are
host-owned platform services: provider endpoints and credentials come from
deployment/org provider configuration, not from agent-authored URLs or
tenant/agent egress policy. Runtime providers own endpoint, auth, and service
headers. Drivers own protocol request construction, streaming parse logic,
retry decision, and error mapping. Shared clients disable redirects and
pin DNS only after private-range validation.

### Message Types

1. **Message**: Provider-agnostic message format
   - `role`: System, User, Assistant, Tool
   - `content`: Text or multipart (text, images, audio)
   - `tool_calls`: Optional tool calls (assistant messages)
   - `tool_call_id`: Optional tool call reference (tool messages)

   **Image Content in Tool Results**: When a tool returns images (via `ToolResultImage`), they become `LlmContentPart::Image` entries in the Tool message's content. Provider-specific handling:
   - **Anthropic**: Tool results with images use array `content` in `tool_result` block (text + image blocks)
   - **OpenAI Chat Completions**: Tool messages include `image_url` content parts alongside text
   - **OpenAI Responses API**: Images not supported in `function_call_output` (text only, images dropped with warning)

2. **LlmCallConfig**: Configuration for LLM calls
   - `model`: Model identifier
   - `temperature`: Optional sampling temperature
   - `max_tokens`: Optional token limit
   - `tools`: Tool definitions
   - `reasoning_effort`: Optional reasoning level (low, medium, high)
   - `speed`: Optional speed selector (flex, default, priority, fast, ultrafast), sent as OpenAI `service_tier`
   - `verbosity`: Optional verbosity selector (low, medium, high), sent as OpenAI `verbosity`
   - `metadata`: Optional request metadata for provider-side correlation
   - `previous_response_id`: Optional OpenAI Responses continuation handle
   - `provider_opaque_context`: Optional losslessly serializable, ordered native
     compact output; mutually exclusive with `previous_response_id`
   - `tool_search`: Optional deferred tool-loading config
   - `prompt_cache`: Optional provider-agnostic prompt-cache config

### Prompt Cache Request Contract

Prompt caching is modeled as request intent on `LlmCallConfig.prompt_cache`. Drivers may ignore it when the provider or model does not support cache controls, but they must not fail a request solely because prompt caching was enabled.

Current provider mappings:

- **OpenAI Responses API**: derives a deterministic cache routing key from stable cache-family inputs. On GPT-5.6 and Astra, auto mode uses implicit caching; opt-in explicit strategy caches the developer-instruction prefix and leaves the conversation suffix unwritten. Explicit mode uses full transcript replay to avoid duplicating developer instructions through stateful continuation. Without developer instructions, explicit mode creates no breakpoint. Older models and non-native gateways retain their existing behavior. The [wire implementation](../../crates/provider/src/openresponses_protocol/mod.rs) owns exact options and breakpoint placement; [OpenAI's cache contract](https://developers.openai.com/api/docs/guides/prompt-caching) owns API semantics.
- **Anthropic**: adds bounded `cache_control: { type: "ephemeral" }` breakpoints to stable/high-value request sections instead of every text block: the tool array, the system prompt, and the **two** most recent stable messages. The pair on the transcript is what makes caching incremental, the newest marks where this turn's history is written, the one behind it sits where the previous turn already wrote, so each turn reads its predecessor's cache instead of re-paying for the transcript. Four total, Anthropic's per-request maximum. Trailing content the caller marks volatile (`volatile_suffix_len`) and mid-conversation system messages are skipped
- **Gemini**: uses `cachedContent` when the config includes an existing cached-content resource name; otherwise the request remains in implicit/default Gemini behavior

`llm.generation.metadata.request_options.prompt_cache` records which provider-specific mode the driver actually attempted.

### Cache Accounting and OpenAI Compatibility

Cache accounting and persistence follow the [disjoint usage contract](../security/usage-tracking.md#disjoint-bucket-convention).

[Compatibility validation](../../crates/provider/src/openai_compat.rs) rejects
unsupported reasoning effort before omission/serialization can hide an invalid
selection. Astra sampling and Chat Completions tool use are rejected before
network I/O. EU Fast/Priority restrictions follow the actual OpenAI regional
endpoint host; residency behind custom proxies is not inferred. These checks
follow the [Astra migration contract](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra).

### Per-Request Headers

`LlmCallConfig.extra_headers` carries caller-supplied HTTP headers for a single
call, distinct from the provider-owned service headers on `ProviderEndpoint`
(which describe the service, not the call). Drivers apply them through
[`merge_request_headers`](../../crates/provider/src/driver_helpers.rs): matching
is case-insensitive and a caller header **replaces** the driver's or provider's
value instead of appending a second copy, so a call can override a protocol
header (for example `anthropic-version`) without producing a duplicate. Headers
that describe the connection rather than the call (`host`, `content-length`,
`transfer-encoding`, `connection`, `upgrade`) are dropped: taking them from
config would corrupt the request, and `host` would repoint an SSRF-guarded
connection at another virtual host.

Applied by the Anthropic driver, the Gemini driver, and the shared Chat
Completions / Open Responses protocols (so every driver built on them).

Callers do not set this by hand in the product: a provider connection stores its
headers and diagnostics opt-in in `settings.request_options`, and
`DriverRegistry::create_chat_driver` wraps the constructed driver so those
options are stamped onto every call's `LlmCallConfig`. See the request-options
section in [providers.md](providers.md#provider).

### Prompt Cache Diagnostics

`LlmCallConfig.cache_diagnostics` asks the provider to explain an unexpected
cache miss instead of leaving `cache_read_tokens` silently at zero. Today only
Anthropic implements it (`cache-diagnosis` beta): the driver sends the beta
flag, serializes `diagnostics.previous_message_id` (explicitly `null` on the
first turn, which is how a request opts in without a prior message to compare
against), and returns the provider's `diagnostics` payload verbatim on
`LlmCompletionMetadata.cache_diagnostics` plus the message id on
`response_id` — the id the next request passes as `previous_message_id`.

The payload shape is provider-owned, so the runtime carries it without
interpreting it. Drivers without a diagnostics protocol ignore the config
rather than failing the call.

When the opt-in comes from a provider connection, `previous_message_id` is
chained automatically from the call's `previous_response_id`, which is the
previous generation of the same turn (tool loops), so the comparison the
provider reports is against the request that immediately preceded it.

### Default `max_tokens` Policy

When `config.max_tokens` is `None`, drivers resolve the default from model profile metadata rather than hardcoding a value:

1. Look up the model via `get_model_profile(provider_type, model_id)`
2. Use `profile.limits.output` as the default `max_tokens`
3. If no profile is found, fall back to a safe default (Anthropic: 16,384; Gemini: 8,192)
4. OpenAI drivers omit `max_tokens` entirely (API decides)

Anthropic requires `max_tokens` in every request (cannot be omitted), so the driver always resolves a value.

**Caller caps and thinking (Anthropic)**: thinking tokens count toward `max_tokens`. An explicit caller
value remains a hard limit on all generated tokens and is serialized unchanged. When thinking does not
fit underneath it — measured against `budget_tokens` for budget-based thinking, or the effort-sized room
adaptive thinking needs, since it carries no budget — the driver omits thinking rather than exceed the
cap. A small cap therefore returns a short answer rather than an empty one. Source:
`crates/drivers/anthropic/src/effort.rs`.

**Append-only history (Anthropic)**: Claude Opus 5.5, Sonnet 5.5, and Fable 5.1 bind each thinking block to the conversation prefix that produced it (`system`, tools, every earlier message), and every model's prompt cache needs the same prefix. Only the leading run of system messages goes into top-level `system`; later system messages stay in place on models whose profile advertises `mid_conversation_system`. Turn-scoped facts and reminders additionally carry `clear_at: "next_user_message"` under beta `mid-conversation-system-clear-at-2026-08-21`, so the transcript retains each copy while the API stops rendering it after the turn. Models without that profile capability retain the user facts and system-fold fallbacks. Requests to Opus 5.5, Sonnet 5.5, and Fable 5.1 set `thinking.block_binding.prefix_mismatch_behavior: "drop_block"` (beta `thinking-binding-controls-2026-08-01`): context management edits earlier history by design, and a dropped block degrades that turn instead of failing it. Drops are logged from `input_transformations`. Source: `crates/drivers/anthropic/src/driver_layout.rs`.

**Always-thinking Claude models**: on families where thinking cannot be disabled (Opus 5.5, Sonnet 5.5, Fable 5.x)
the driver always sends an explicit effort. No caller effort sends the profile default, and an explicit
`none` sends `low`, the closest level the API accepts. Requests that end on an assistant turn (prefill)
are rejected with a configuration error before any network call on every adaptive-thinking family,
because the API answers them with a 400.

**Stale profile fallback**: If the Anthropic API returns 400 because `max_tokens` exceeds the model's actual limit (e.g., stale profile data), the driver retries once with 16,384 and logs a warning to update the model profile. If the retry also fails, the error propagates normally.

Agents can override `max_tokens` via agent config; the driver preserves explicit limits as resource
guardrails.

3. **Message Extended Fields**:
   - `reasoning`: Ordered provider reasoning artifacts for this assistant turn

### Reasoning Support

Reasoning is modelled as an ordered list of provider artifacts, not as text on
the message. The shape lives in `crates/provider/src/reasoning.rs`.

Ordering is the reason it is a list rather than a pair of fields. Providers
interleave reasoning with text and tool calls, and each one requires its
artifacts replayed in the position it issued them: Anthropic verifies every
thinking block against the signature for *that* block, OpenAI keys reasoning
items by the id it issued and expects each adjacent to the item it precedes, and
Gemini binds a thought signature to one specific function call. Flattening any
of that into a single per-message field produces artifacts the provider rejects
or ignores.

Each artifact separates three things that were previously conflated: readable
text (raw chain-of-thought vs a provider-curated summary vs withheld), opaque
replay state (signature / encrypted payload), and identity (provider, item id,
bound tool call). Only readable text is ever rendered or published; replay state
is carried verbatim and never leaves the driver boundary.

Anthropic has two thinking request forms, selected per model family by the driver. Recent Claude families (Fable 5.x, Opus 5.5/5/4.8/4.7, Sonnet 5.5/5, and the 4.6 family) take adaptive thinking (`thinking.type = "adaptive"` plus `output_config.effort`); the budget-based `budget_tokens` form is removed on Fable 5.x, Opus 5.5/5/4.8/4.7, and Sonnet 5.5/5 and returns 400 there. Older Claude models keep budget-based extended thinking. The family list lives in `crates/drivers/anthropic/src/driver.rs` and must stay in sync with the adaptive-thinking profiles in `crates/model-profiles/src/profiles.rs`.

#### Stream Events

When `reasoning_effort` is configured, drivers emit two reasoning events:
- `ReasoningDelta { delta, summary }` -- incremental readable reasoning. Always
  the reasoning channel, never assistant text. `summary` distinguishes a
  provider-curated summary from raw chain-of-thought so consumers can label what
  they show instead of guessing.
- `ReasoningItem(ReasoningContentPart)` -- one completed artifact, carrying both
  its readable text and the opaque state needed to replay it.

#### Multi-turn Reasoning Contract

Reasoning context must be preserved across turns, and how differs per provider.
The runtime carries whatever the provider issued and hands it back unchanged;
each driver knows its own wire form.

| Provider | Readable text | Replay state | Scope |
|---|---|---|---|
| Anthropic | chain-of-thought, or withheld (`redacted_thinking`) | signature per block | one block |
| OpenAI Responses | curated summary | `encrypted_content` keyed by `rs_…` id | one item |
| Gemini | thought parts | `thoughtSignature` | a turn, or one function call |
| Chat Completions | `reasoning_content` | none | n/a |

Two provider requirements are easy to miss and silently degrade when unmet:
OpenAI returns `encrypted_content` only when the request opts in via
`include`, and Gemini returns thought parts only when `thinkingConfig` sets
`includeThoughts`. Without those, the model still reasons but nothing is
replayable and nothing reaches the reasoning channel.

Provider-specific wire format details live in the driver implementations:
- `crates/drivers/anthropic/src/driver.rs` -- thinking form selection (adaptive vs budget-based), beta headers, per-block signature capture, message ordering
- `crates/provider/src/openresponses_protocol/mod.rs` -- reasoning config, `include`, encrypted content, reasoning item ids
- `crates/drivers/gemini/src/driver.rs` -- thinking budget, thought parts, thought signatures

#### Reasoning Guard Logic

Reasoning parameters are validated at two levels to prevent API errors from non-thinking models:

1. **ReasonAtom (`reason.rs`)**: Before building the LLM call config:
   - Strips `reasoning_effort` when value is `"none"` (no-op)
   - Looks up model profile via `get_model_profile()`, if `reasoning: false`, strips reasoning_effort with a warning log
   - Unknown models (no profile) pass through to let the API decide

2. **Driver level**: Both OpenAI drivers filter out `effort: "none"` before sending:
   - Responses API: omits `reasoning` object entirely
   - Chat Completions API: omits `reasoning_effort` field

   The Anthropic driver additionally drops `temperature` for models whose
   profile marks it unsupported (sampling parameters are removed from Opus 4.7
   on; the API rejects them with a 400).

The UI also prevents setting reasoning on non-thinking models (checks `profile.reasoning` and `profile.reasoning_effort`).

### Speed (Service Tier)

The speed selector maps to OpenAI's `service_tier` request parameter: `flex` trades latency for batch-rate pricing, `priority` buys faster and more consistent latency at a premium, `default` pins the standard tier. `fast` is OpenAI's current name for `priority` (Fast mode); the API accepts both and older models echo `priority` for either, so the two are one tier wherever a profile is checked. `ultrafast` is a separate premium tier (6x Standard, GA for GPT-6 Astra only at the time of writing). The API rejects values outside the closed set at message creation. It is resolved per turn from the latest user message's `controls.speed` and guarded like reasoning effort: ReasonAtom strips the value (with a warning log) when the model profile carries no `speed` config, and unknown models pass through. When unset, the field is omitted so the provider keeps its default (`auto`) routing.

A tier is model-gated at OpenAI, so the OpenAI drivers' pre-flight check rejects a tier the model's profile does not list with a configuration error (`default` is always allowed) instead of letting the provider answer 400; on the session path ReasonAtom has already dropped it. Cost follows the tier that served the call, not the one requested: the Responses driver carries the response's echoed `service_tier` on the completion metadata, and the price-table estimate scales by that tier's `cost_multiplier` from the profile (recorded only where OpenAI prices the tier as a flat multiple of Standard: GPT-6 series Flex 0.5x, Fast 2x, Ultrafast 6x). A ramp-limited premium request that degrades to `default` is therefore priced at Standard. The Chat Completions driver does not surface the echoed tier yet and prices at Standard.

Per-model availability lives in the model profile's `speed` config, sourced from OpenAI's official tier tables, the API pricing page for Flex, the Priority-processing docs for first-party priority models, and the specialized Codex priority table (models without a tier row get no config). Profiles mask the config for every provider surface except first-party OpenAI, Azure and gateways have their own capacity models. Both OpenAI drivers (Responses and Chat Completions) serialize the value verbatim as `service_tier`; other drivers ignore it.

### Verbosity

The verbosity selector maps to OpenAI's `verbosity` request parameter (`low`, `medium`, `high`): a hint for how expansive the final answer should be, orthogonal to reasoning effort (which tunes how much the model thinks). The API rejects values outside the closed set at message creation. It is resolved per turn from the latest user message's `controls.verbosity` and guarded like speed: ReasonAtom strips the value (with a warning log) when the model profile carries no `verbosity` config, and unknown models pass through. When unset, the field is omitted so the provider keeps its default (`medium`).

The two OpenAI drivers place the field differently: the Responses API nests it under `text.verbosity`, while the Chat Completions API takes it as a top-level `verbosity` field. Other drivers ignore it. Per-model availability lives in the model profile's `verbosity` config (currently the GPT-5.5, GPT-5.6, and GPT-6 Astra series).

### Structured Output

A call can require its answer to validate against a JSON Schema: `LlmCallConfig.response_format` carries a provider-neutral `ResponseFormat` (see `crates/provider/src/structured_output.rs`), and each driver maps it to its native control. The OpenAI Responses driver sends it as `text.format` (next to `text.verbosity`), the Chat Completions driver as the top-level `response_format`. Strict adherence is the default because a caller asking for a schema wants it enforced, not approximated; `non_strict()` opts out for schemas that cannot meet OpenAI's strict-mode rules.

A driver declares support through `ChatDriver::supports_response_format`, which defaults to false and must be forwarded by wrapper drivers like the other capability hooks. The provider refuses a call carrying a format on a driver without support, with a configuration error, before any request is sent. Silently dropping the schema would return free text to a caller that believes it holds validated JSON. The simulator reports support, so offline tests and examples can exercise the path. Anthropic (`output_config.format`) and Gemini (`responseJsonSchema`) have native equivalents and are the next drivers to wire.

Scope: single completions (Framework `Completion::response_format`, evals, utility-style calls). Agent turns do not set it; an agent's final message stays prose, and tools remain the way an agent returns structured data.

### Background Mode (OpenAI Responses)

A long `xhigh`/`max` reasoning call on the OpenAI driver runs with `background: true` and `store: true`, so the response lives at OpenAI rather than on one HTTP connection. The driver tracks the response id and each event's `sequence_number`; when the connection fails or closes before a terminal event, it re-attaches with `GET /responses/{id}?stream=true&starting_after=N` and the parser sees one gapless stream. The call is never posted twice, so a dropped connection no longer loses or re-bills minutes of reasoning. Dropping the stream before a terminal event (turn cancellation, the stall timeout, a worker shutdown) sends `POST /responses/{id}/cancel`, because a background response otherwise keeps generating and billing with nobody reading it.

Policy lives in `crates/provider/src/openresponses_protocol/background.rs`: on for `xhigh`/`max`, forced on or off by the `openai/background` driver option, and limited to OpenAI and Azure hosts (OpenRouter and custom gateways have no resume API). Background mode requires stored responses, so it is not zero-data-retention compatible: a 400 naming the background fields retries once in the foreground, and ZDR deployments can set the option to `false`.

Re-attaching survives a worker restart (EVE-1134). A durable host passes a `BackgroundCallContext` (`crates/provider/src/background_call.rs`) on `LlmCallConfig`: a journal for the response id and an explicit turn-cancel signal. The id is saved when `response.created` arrives, before the parser sees any event, into the turn's native-async checkpoint (`crates/host/src/background_call.rs`; each write takes and releases the turn lease, so no lease is held across the call). The durable retry of the same call re-attaches with `GET /responses/{id}?stream=true` from the first event, because its parser starts empty, instead of posting again. The record carries a fingerprint of the request (metadata excluded, since it holds per-attempt ids); a record for a different request is cancelled rather than resumed, and a record that can no longer be fetched falls back to posting. The record is cleared at the terminal event.

With a journal, dropping the stream no longer cancels the response: the drop may be a worker shutdown or a stall whose retry re-attaches. Cancellation is explicit instead: the worker heartbeat reports a cancelled workflow to the worker that still owns the task (see [durable execution](../operations/durable-execution-engine.md#task-heartbeat-cancellation)), and the driver sends `POST /responses/{id}/cancel` while it is still reading the stream. Ownership loss never cancels, because the next owner is re-attaching to that response. Without a journal (embedded hosts, or a journal write that fails) the in-process behaviour stays: an abandoned response is cancelled on drop.

Remaining gaps: a worker that dies after OpenAI accepted the POST but before `response.created` was saved still re-posts; a response abandoned by a turn that then fails without retrying runs to completion unread; and in the native-async path the coordinator holds the turn lease, so the journal is unavailable there and the call keeps the in-process behaviour.

### WebSocket Transport (OpenAI Responses)

The Open Responses driver can stream a call over OpenAI's Responses WebSocket mode instead of SSE, keeping one socket open across the turns of a tool loop. It is opt-in (the `openai/websocket` driver option, or `OpenAIChatDriver::with_websocket_transport`), offered by the OpenAI driver for `api.openai.com` only, never used for background-mode calls, and falls back to SSE whenever the socket fails before the first response event. The wire contract, commit point and reuse rules are in [OpenAI Responses WebSocket Transport](openai-responses-websocket.md).

### Completion Metadata

`LlmCompletionMetadata` returned on stream completion. Token buckets are
disjoint (drivers normalize inclusive providers at the boundary; see
[usage-tracking](../security/usage-tracking.md#disjoint-bucket-convention)):
- `total_tokens`: Total tokens used (non-cached prompt + cache + completion)
- `prompt_tokens`: Non-cached input tokens (cached reads excluded)
- `completion_tokens`: Output tokens
- `cache_read_tokens`: Tokens from cache, additive (not part of `prompt_tokens`)
- `cache_creation_tokens`: Tokens written to cache (Anthropic)
- `model`: Actual model used
- `finish_reason`: Why generation stopped
- `response_id`: Provider generation id (Anthropic message id, OpenAI response id)
- `cache_diagnostics`: Provider prompt-cache diagnostics, verbatim, when requested
- `reasoning_tokens`: Reasoning tokens billed *inside* `completion_tokens`, not
  additive to them, when the provider reports the breakdown
- `request_body`: The driver's serialized request body, only when the call set
  `LlmCallConfig::capture_request`. See TM-LLM-039: it carries the whole prompt,
  so it is off by default and never enabled on a caller's behalf

### Turn Collection and Per-Call Limits

Folding a provider stream into one finished turn is one shared loop,
`turn_collector::collect_turn` ([source](../../crates/provider/src/turn_collector.rs)),
not a per-driver or per-embedder reimplementation. The `ChatDriver`
non-streaming default runs through it, so an agent turn and a direct call fold
identically.

The contract that matters:

- reasoning is folded from `ReasoningItem` events only; `ReasoningDelta` is live
  progress that repeats the same text, so folding both would double it;
- a stream that ends without its terminal `Done` is reported on
  `CollectedTurn::complete` rather than silently passing for a whole turn — its
  metadata is a default, not the provider's answer. Callers that need usage and
  a finish reason to be real set `TurnLimits::require_terminal_event`;
- `TurnLimits` on `LlmCallConfig::limits` bound the turn (whole-turn timeout,
  first-event timeout, accumulated-byte cap). Unbounded by default so no driver
  changes behavior; `limit_stream` applies the same bounds for callers that
  consume events themselves. See TM-DOS-039.

### Realtime Voice Driver

Realtime voice does not use `ChatDriver::chat_completion_stream()` because a
voice connection is a long-lived bidirectional provider session, not a bounded
request/response generation. Voice support adds a separate Realtime provider
adapter owned by the server voice domain. See [voice.md](../operations/voice.md).

V1 adapter requirements:

- OpenAI only, model `gpt-realtime-2`.
- Mint client secrets through `/v1/realtime/client_secrets` or proxy SDP through
  `/v1/realtime/calls`.
- Prefer WebRTC proxy bootstrap for browser voice so the server can capture the
  provider call ID and open a sideband control channel.
- Set `OpenAI-Safety-Identifier` on the trusted server request that creates the
  provider realtime session.
- Open a sideband WebSocket when a provider call ID is known.
- Convert effective Everruns capabilities into provider tool definitions and
  execute tool calls through the normal server-side capability path.
- Map provider transcript and lifecycle events into Everruns `voice.*`,
  `input.message`, `output.message.*`, and `tool.*` events.
- Never log or persist standard API keys, client secrets, raw SDP, or raw audio.

`gpt-realtime-2` model profiles should mark the model as a realtime reasoning
voice model with configurable reasoning efforts (`minimal`, `low`, `medium`,
`high`, `xhigh`). It should not appear in normal text chat model pickers unless
the UI is explicitly rendering a voice-capable picker.

## Error Handling Flow

```mermaid
sequenceDiagram
    participant R as ReasonAtom
    participant D as ChatDriver
    participant A as API

    R->>D: chat_completion_stream()
    D->>A: HTTP Request
    A-->>D: Error Response

    alt Request Too Large
        D->>D: Detect via is_*_request_too_large()
        D-->>R: Err(RequestTooLarge(msg))
        R-->>User: "Conversation too long..."
    else Rate Limited (429)
        D-->>R: Err(Llm(msg))
        R->>R: is_rate_limited() = true
        R-->>User: "Rate limited by the AI provider..."
    else Auth Error (401/403)
        D-->>R: Err(Llm(msg))
        R->>R: is_auth_error() = true
        R-->>User: "Misconfiguration, contact support..."
    else Server Error (5xx)
        D-->>R: Err(Llm(msg))
        R->>R: is_server_error() = true
        R-->>User: "Provider experiencing issues..."
    else Other Error
        D-->>R: Err(Llm(msg))
        R-->>User: "Error processing request..."
    end

    Note over R: Full error logged server-side
```

### Automatic recovery contract

Provider failures are classified at the provider boundary before the runtime
decides whether to recover. Transport loss, a stream that makes no output
progress, overload, ordinary rate limiting, and retryable server failures may
be retried. Invalid credentials, exhausted billing quota, unavailable models,
invalid/unsafe requests, and long-horizon usage limits fail fast with their
specific decision.

Recovery is bounded by both attempts and elapsed wall-clock time. Backoff is
exponential with jitter, honors reasonable provider retry hints, and lower
driver layers report consumed retries so the reason loop cannot multiply the
budget. A retry is permitted only before assistant output commits. Act phases
and completed tool results remain outside the retry boundary, preserving
exactly-once side effects and provider-visible tool-call/output pairing.

For a stateful Responses continuation rejected because provider state is
missing a tool call or output, the driver may retry once without the opaque
continuation handle. That fallback replays the locally persisted transcript
through the same atomic tool-pair repair used by stateless requests; it never
re-executes a completed tool.

## Implementation Location

| Component | Location |
|-----------|----------|
| ChatDriver trait | `crates/provider/src/driver_registry.rs` |
| AgentLoopError | `crates/provider/src/error.rs` |
| OpenAI driver | `crates/drivers/openai/src/driver.rs` |
| Open Responses protocol | `crates/provider/src/openresponses_protocol/mod.rs` |
| Chat Completions protocol | `crates/provider/src/openai_protocol.rs` |
| Anthropic driver | `crates/drivers/anthropic/src/driver.rs` |
| Gemini driver | `crates/drivers/gemini/src/driver.rs` |
| Bedrock driver | `crates/drivers/bedrock/src/driver.rs` |
| Microsoft MAI driver | `crates/drivers/mai/src/driver.rs` |
| Fireworks AI driver | `crates/drivers/fireworks/src/driver.rs` |
| Meta Model API driver | `crates/drivers/meta/src/driver.rs` |
| Cloudflare AI Gateway driver | `crates/drivers/drivers/src/cloudflare.rs` |
| Vercel AI Gateway driver | `crates/drivers/drivers/src/vercel.rs` |
| Error handling | `crates/engine/src/execution/reason.rs` |

## OpenAI Driver Variants

The OpenAI crate provides three driver implementations:

1. **OpenAIChatDriver** (`ProviderType::OpenAI`)
   - Uses Open Responses API (https://www.openresponses.org/)
   - Recommended for new projects
   - Wraps `OpenResponsesProtocolChatDriver`

2. **OpenAICompletionsChatDriver** (`ProviderType::OpenAICompletions`)
   - Uses Chat Completions API (`/v1/chat/completions`)
   - For backward compatibility with legacy integrations
   - Wraps `OpenAIProtocolChatDriver`

3. **OpenRouterChatDriver** (`ProviderType::OpenRouter`)
   - Uses OpenRouter's OpenAI-compatible Responses API
   - Defaults to `https://openrouter.ai/api/v1/responses`
   - Wraps `OpenResponsesProtocolChatDriver` with the OpenRouter provider profile

The Responses drivers share the same base URL handling and can work with
OpenAI-compatible endpoints while keeping provider-specific request features
gated by the resolved provider type.

Both OpenAI protocol variants adapt model-facing tool JSON Schemas at the
provider boundary. Supported constraints pass through unchanged. Regex
lookaround, which OpenAI rejects, is translated when an equivalent supported
pattern is known (including the practical Zod email validator); otherwise only
that unsupported model-facing pattern is omitted and the tool remains the
authoritative validation boundary. This keeps third-party MCP discovery live
without mutating or weakening unrelated schemas.

## Meta Model API Driver (`everruns-meta`)

Meta Model API serves Muse models at `https://api.meta.ai/v1`. The dedicated
driver uses the shared Responses protocol, preserves server-managed continuation
through `previous_response_id`, and discovers models from the host-gated
`/v1/models` endpoint. Profile gating enables Meta's native message phases and
hosted tool search only on the direct `meta` surface; gateway aliases fall back
to client-side transcript replay and tool search.

## Gateway Drivers (`everruns-drivers`)

Cloudflare AI Gateway and Vercel AI Gateway front many upstream vendors behind one
OpenAI-compatible endpoint, so neither needs a wire implementation. Both live as feature-gated
modules in one crate rather than a package each; the crate's
[README](../../crates/drivers/drivers/README.md) owns the membership rule and when a vendor
graduates out of it.

Which endpoint each speaks was settled by measurement, and the losing options fail in ways worth
recording because the documentation does not predict them.

Vercel serves the Open Responses spec at `/v1/responses`, so it uses the shared Open Responses
driver. Model ids are namespaced (`provider/model`), and the gateway's own `/models` catalog is
synced, host-gated like the other drivers.

Cloudflare uses Chat Completions on its account-scoped AI REST API. Its `/compat` endpoint is
deprecated by Cloudflare for single-model calls, and of the four formats the REST API offers only
Chat Completions both serves the whole catalog and streams:

| Endpoint | Why not |
| --- | --- |
| `/ai/v1/responses` | Rejects Workers AI models: a `@cf/` model answers HTTP 400 asking for `prompt` or `messages`, because the gateway does not translate them into the Responses shape |
| `/ai/run` | Cannot stream. With `stream: true` it answers `content-type: application/json` and an empty `{"result":{}}` body — no events, HTTP 200, no error |
| `/ai/v1/messages` | Excludes Workers AI models |

Cloudflare's base URL is derived from the account id rather than hand-written, since its shape is
fixed; the gateway is selected by the `cf-aig-gateway-id` header rather than the URL, and the
account's own API token (Account > Workers AI > Read) bills the whole call, so no upstream vendor
keys are involved.

Both gateways support discovery, and both gate it on the vendor's own host so a proxy base URL is
never probed (the OpenRouter/Meta/Fireworks posture). They differ in what a catalog can even mean:

| Gateway | Catalog | Completeness |
| --- | --- | --- |
| Vercel | `/models`, the OpenAI-compatible listing on the same base URL | Complete: every `vendor/model` the gateway routes |
| Cloudflare | `ai/models/search`, a sibling of `v1` rather than a path under it | Partial: Workers AI (`@cf/`) only, filtered to the text-generation task |

Cloudflare's listing cannot be complete. Which third-party models an account reaches depends on
what it can bill, and no endpoint enumerates that, so those ids are still added by hand. The
partial list is returned anyway because the `@cf/` ids are the awkward ones to type, and they come
with the context window and `function_calling` flag the catalog advertises. One trap: that
endpoint's `result_info.total_count` does not describe the result — an account served 69 models in
one page is told the total is 321, and page 2 is empty — so the driver reads the page it is given
rather than paginating on that number.

## Microsoft MAI Driver (`everruns-mai`)

Microsoft MAI models (e.g. `mai-code-1-flash`) are served via Azure AI Foundry
behind an OpenAI-compatible Chat Completions API. The `everruns-mai` crate is a
standalone, crates.io-publishable provider crate that wraps the shared
`OpenAIProtocolChatDriver` (from `everruns-provider`) and tags it with
`DriverId::Mai`. Model ids resolve
to the Microsoft-vendor profiles in `crates/model-profiles/src/profiles.rs` (the
`MICROSOFT_MAI` surface).

### Catalog Fallback for Unrecognized Endpoints

A vendor driver returns `Ok(None)` for an endpoint it does not recognize, and
that gate stays where it is. It is a **credential boundary**, not merely a
capability check: the base URL is org-configured and the listing URL is
*derived* from it, so anything the driver sends there resolves the provider's
key against a host that may only look like the vendor
(`api.openai.com.evil.example`, `resource.openai.azure.com@evil.example`,
`evil.example/api.openai.com`). Attaching a generic OpenAI-compatible fallback
to a vendor driver would hand the key to exactly those hosts, so it is not
done. `public_discovery_gates_both_protocols_before_accessing_credentials`
pins this with those cases.

A caller that *does* want a catalog from a host it trusts — a proxy, a
gateway, its own server — calls
`model_discovery::list_openai_compatible_models_best_effort` itself. That is
the deliberate step the trust decision deserves. It is best-effort: the caller
asked *whether* a catalog exists, so an endpoint that serves none answers "no"
rather than erroring, and it reuses `validate_safe_url` and the shared
DNS-pinned client (TM-API-013).

Enrichment keeps the capability metadata: `DiscoveredProviderModel::profile`
carries the curated registry profile merged with whatever the provider's API
reported, so a model the registry has never heard of still arrives with the
limits its provider advertises. Curation wins where both speak, because it
holds what an API never returns (prices, knowledge cutoff).

### Model Discovery

Foundry exposes an OpenAI-compatible `/models` endpoint, but it is **bare**
(`id`/`created`/`owned_by` only, no capabilities, limits, or cost), exactly
like OpenAI's. So `list_models` syncs model/deployment **ids** with
`discovered_profile: None` and relies on the built-in profile registry to supply
capabilities by matching the id at sync time (the OpenAI/Azure OpenAI pattern).
Rich capability metadata lives only in the separate Foundry model *catalog* API
(different host/auth, reflects the catalog not the deployment) and cost is never
provider-reported, so neither is used for profiling. The discovery request is
authenticated with the same runtime `ProviderAuth` as chat (so it works for both
api-key and OAuth) and is gated to recognized Foundry hosts
(`*.services.ai.azure.com` / `*.openai.azure.com`); custom proxy URLs return
`None`. Project-scoped Foundry endpoints (`.../api/projects/<project>`) expose
`/openai/v1/chat/completions` but **not** a `/models` catalog, a 404 (or 501)
on the listing endpoint is treated as "discovery not supported" (`Ok(None)`),
not an error, so model sync degrades gracefully. **Caveat:** Azure deployment
names are operator-chosen, so a deployment id that does not match a known
profile (e.g. `mai-code-1-flash`) falls back to a minimal profile, the same
limitation Azure OpenAI has.

### Authentication storage and end-to-end OAuth

`MaiAuth::from_driver_config` resolves auth in two ways. In-process / embedder
callers may pass an Entra OAuth block in `ProviderMetadata.extra`. Server-stored
providers carry their secret as the driver's declared typed credential fields,
**either** a plain Azure AI Foundry `api_key` **or** discrete Entra OAuth fields
(`tenant_id`, `client_id`, `client_secret`, optional `scope` / `authority`),
which the operator enters as separate inputs (no hand-authored JSON). These
fields are assembled into the encrypted credential document and parsed back into
the typed `DriverConfig::credentials` map (`parse_credential_document`), so OAuth
works end-to-end for **both** chat execution and model sync with no
provider-metadata forwarding, and the fail-closed key-resolution contract is
preserved (a stored credential is still required; nothing falls back to env).
Existing rows that stored OAuth as a JSON document keep resolving, since that
document parses into the same typed fields.

### Provider-owned authentication

MAI deployments accept either an Azure AI Foundry **API key** (`api-key`
header) or a Microsoft **Entra ID (OAuth)** bearer token. Both are runtime
`ProviderAuth` implementations attached to a provider over the reusable Chat
Completions protocol. `ProviderAuth::headers` is async and receives the method,
URL, service headers, and exact serialized body on every HTTP attempt. This
supports refreshable OAuth and body-aware signing without teaching a protocol
driver about service identity. Bedrock keeps the AWS SDK client in
provider-owned `BedrockAuth`, preserving SDK SigV4 signing and event-stream
framing.

The four request boundaries are deliberately separate and compose as follows:

- **Credential storage / resolution** (`CredentialProvider`,
  `crates/core/src/credential_provider.rs`) resolves a *static*
  `ProviderCredentials` (`api_key`, `base_url`) **before** the driver is built.
  It cannot mint or refresh request-time tokens.
- **Request auth** (`ProviderAuth`) runs **per HTTP attempt** and may return
  multiple headers. This is the boundary for static, refreshable, and signed auth.
- **Request decoration** (`OpenResponsesRequestExtension`, Open Responses only)
  layers provider-specific *non-auth* body fields and headers (routing,
  attribution, `session_id`, `OpenAI-Beta`, `originator`, account ids). It must
  not set auth headers.
- **Precedence:** the driver applies decoration headers first, then inserts the
  resolved auth header, so **auth always wins on a header-name conflict**.

The MAI crate ships two providers built on that hook:

- **API key**: emits `("api-key", <key>)`.
- **Entra ID OAuth**: client-credentials grant against
  `{authority}/{tenant_id}/oauth2/v2.0/token` (default scope
  `https://cognitiveservices.azure.com/.default`). Minted bearer tokens are
  cached and refreshed ~120s before expiry.

`MaiAuth::from_driver_config` selects the scheme: an Entra OAuth block in
`ProviderMetadata.extra` (`tenant_id`, `client_id`, `client_secret`, optional
`scope`/`authority`) wins; otherwise the configured `api_key` is used; neither
present is a fail-closed configuration error. Because OAuth needs no `api_key`,
`DriverId::Mai` is exempt from the registry's mandatory-api-key check (like
`External`). New schemes (managed identity, workload identity federation) are
additive: implement `ProviderAuth` without touching the protocol driver.

### Model Discovery and Capability Profiles

`list_models` runs only for hosts known to expose a usable `/models` endpoint
(`api.openai.com`, Azure OpenAI, and `openrouter.ai`); other custom base URLs
(self-hosted proxies) return `None` and are not synced.

OpenAI's `/models` response is bare (`id`, `created`, `owned_by`) and yields no
profile. OpenRouter returns richer metadata, notably a `supported_parameters`
array, which the driver parses into an `LlmModelProfile`: `reasoning` (or the
legacy `reasoning_effort` alias) maps to `profile.reasoning` plus a low/medium/high
`reasoning_effort` config, and `tools`/`response_format`/modalities map to the
corresponding capability flags. This matters because most OpenRouter models
(e.g. NVIDIA Nemotron) have no hardcoded profile; without discovery the UI cannot
tell they support reasoning and hides the effort selector. The discovered profile
is persisted via the model-sync pipeline and surfaces on existing rows on the next
sync. Cost is left to hardcoded profiles.

OpenRouter-specific routing controls (`models`, `route`, `provider`, and `plugins`) are
represented by `OpenRouterRoutingConfig` in the OpenRouter driver crate and carried
opaquely on `LlmCallConfig.driver_options` under the `openrouter/routing` key. The Open
Responses protocol driver serializes them only when the resolved provider type is
OpenRouter, so ordinary OpenAI-compatible requests do not receive non-standard OpenRouter
routing extensions.

For OpenRouter session tracking, the driver forwards the Everruns session id as a
top-level `session_id` request field, sourced from `config.metadata["session_id"]`. This
groups all generations from one Everruns session into a single session in the OpenRouter
dashboard. Like the routing controls, it is emitted only for OpenRouter requests; direct
OpenAI / Azure ignore the field, so it is omitted there. The same id also rides along
inside the `metadata` map for provider-side correlation, the top-level field is what
OpenRouter's session feature reads.

`OpenRouterRoutingConfig.plugins` exposes optional OpenRouter plugin activations:

- **Web-search plugin** (`OpenRouterWebSearchPlugin`): instructs OpenRouter to retrieve web
  search results before the model sees the prompt. Accepts `max_results: Option<u32>` and
  `search_prompt: Option<String>`.
- **File-reader plugin** (`OpenRouterFilePlugin`): instructs OpenRouter to read and attach
  file contents before the prompt. No options yet.

### OpenRouter Workspace Policy Inspection

The `openrouter_workspace` capability contributes two host-facing tools for reading workspace constraints and detecting incompatibilities before routing decisions are made:

- **`inspect_openrouter_workspace`**: calls OpenRouter's `/api/v1/auth/key` endpoint and returns structured `OpenRouterKeyInfo` (label, usage, limit, tier, rate-limit). The raw API key is never included in the response.
- **`check_openrouter_policy_compatibility`**: fetches workspace info and compares it against caller-supplied routing parameters (`min_remaining_budget_usd`, `requires_paid_features`), returning a `PolicyCompatibilityReport` with a list of `WorkspacePolicyDrift` entries.

Drift kinds:
- `budget_exhausted`, workspace spend cap is reached.
- `budget_below_threshold`, remaining budget is below the requested threshold.
- `free_tier_restriction`, workspace is on the free tier but paid-tier features were requested.

The check is read-only and does not modify workspace state. Results should be treated as advisory; operators are responsible for acting on detected drifts.

Plugins serialize as a `plugins` array on the wire, each entry carrying an `"id"` field
(`"web"` or `"file"`) plus any plugin-specific options. The field is omitted entirely for
non-OpenRouter providers and when no plugins are configured.

### OpenRouter Server Tools

OpenRouter "server tools" (beta) are provider-executed tools the model may invoke during a
request (`web_search`, `web_fetch`, `datetime`, `image_generation`, `apply_patch`, `fusion`,
`advisor`, `subagent`). Unlike client-executed function tools and unlike the `plugins`
mechanism (which always runs *before* the prompt), server tools are model-decided and run
**server-side by OpenRouter**, which loops internally and returns the final answer. The agent
loop therefore never dispatches them; the only client-visible artifact is
`usage.server_tool_use`.

- **Request shape**: each enabled tool is appended to the request `tools` array (alongside any
  function tools) as `{"type": "openrouter:<name>"}`, with optional tool-specific
  `parameters` (e.g. web_search `max_results`). `OpenRouterRoutingConfig.server_tools`
  (`OpenRouterServerTool` / `OpenRouterServerToolKind`) is the typed representation;
  `OpenRouterRequestExtension` appends the entries. Emitted only for OpenRouter requests.
- **Runtime configuration**: the `openrouter_server_tools` capability exposes per-agent
  toggles (`config_schema`) and compiles the selection into the `openrouter/routing`
  driver option during capability collection, the same path
  `prompt_caching` uses. It contributes *request intent only*, no executable tools. Because
  `web_search` and `web_fetch` run outside Everruns' egress boundary, the capability is
  `RiskLevel::High` and uses the admin-only assignment gate. Enabling it on a
  non-OpenRouter agent is a harmless no-op (the routing config is ignored).
- **Usage accounting (follow-up)**: `usage.server_tool_use` is not yet surfaced in
  `LlmCompletionMetadata`; capturing it for cost tracking requires extending that shared
  cross-provider struct and is tracked separately.

### OpenAI Hosted Tools

OpenAI hosted tools (EVE-1115: `web_search`, `code_interpreter`, `shell`, `file_search`, `mcp`) are the OpenAI counterpart of OpenRouter
server tools: model-decided, executed inside the response, never dispatched by the agent loop.

- **Contract**: `everruns_provider::openai_hosted_tools` owns the typed selection and the
  `openai/hosted_tools` driver option; the `openai_server_tools` capability contributes it.
- **Rendering**: the Open Responses driver appends the wire entries only when built
  `with_hosted_tools(true)` (the OpenAI and Azure OpenAI driver). Any other Responses endpoint
  returns a configuration error instead of sending a request without them.
- **Loud off-provider**: unlike OpenRouter's no-op, the reason step fails the turn when the
  option reaches a provider outside `HOSTED_TOOLS_DRIVER_IDS`. An agent configured to search
  must not quietly answer from memory.
- **Stream**: hosted call item frames (`*_call`, see `hosted_call_tool`) become `LlmStreamEvent::HostedToolCall` progress
  (never `ToolCalls`), which the engine persists as `tool.hosted_call` for the activity UI;
  no `tool.completed` is emitted because that event carries tool results into replay. `Done`
  counts calls in `hosted_tool_calls`, and the engine adds their list price
  (`hosted_call_price_usd`) to the estimated cost. The engine counts the option as
  provider-executed, so a mid-stream failure is not reissued.
- **Containers**: `code_interpreter` and `shell` use OpenAI's auto container, which is not the
  session sandbox. They stay unpriced in the estimate because OpenAI bills per container
  session, and one container serves calls across turns.
- **MCP approvals**: an `mcp_approval_request` item is the one hosted interaction that needs a
  person. The driver surfaces it as a synthetic `openai_mcp_approval` tool call with no tool
  definition; ActAtom routes it down the client-side path (`act_hooks::runs_on_server`), so the
  turn parks on `tool.call_requested` and resumes through `tool-results`. On replay the call and
  its result become `mcp_approval_request` / `mcp_approval_response` items
  (`replay_mcp_approvals`), and the delta window treats the request as prior output.
- **MCP credentials**: config never holds one. An entry names a registered Everruns MCP server
  (`mcp_server`) instead of a URL; `everruns_provider::hosted_mcp::HostedMcpDriver` wraps the
  turn driver (`StoreTurnContextResolver::with_hosted_mcp_resolver`, fed by
  `RuntimeHostAdapter::hosted_mcp_resolver`) and fills URL and headers per call from the same
  lookup `mcp_*` execution uses, so OAuth refreshes land on the next request. The headers live
  only in that call's cloned config below the engine, so they never reach events. A missing
  grant or secret-bound parameters fail the turn: OpenAI calls the server itself, so there is no
  tool call to answer `connection_required`. Hosts without the hook (the embedded runtime)
  refuse registered entries.

### OpenRouter Capacity Strategy

`OpenRouterRoutingConfig.capacity_strategy` lets callers express an organization-level
allocation intent without encoding raw OpenRouter provider flags directly:

| Variant | Behaviour |
|---------|-----------|
| `SharedCapacity` (default / `None`) | No changes, uses OpenRouter's shared capacity pool as-is. |
| `ByokFirst` | Sets `provider.allow_fallbacks = true` (if not already set) so OpenRouter tries BYOK providers first and falls back to shared capacity when exhausted. |
| `ByokOnly` | Requires `provider.only` to list at least one BYOK provider slug; sets `provider.allow_fallbacks = false` so no fallback to shared capacity occurs. Returns an error at request time if `provider.only` is empty. |

The strategy is compiled into the low-level `OpenRouterProviderRouting` by
`apply_capacity_strategy()` before the request is serialized. The
`capacity_strategy` field itself is never sent to the API.

`is_empty()` treats `SharedCapacity` and `None` as equivalent (both represent the
default, no-op state) and returns `false` for `ByokFirst`/`ByokOnly` so those
configs are not silently discarded.

### OpenRouter Routing Presets

`OpenRouterRoutingConfig.presets` accepts a list of `OpenRouterRoutingPreset` values that
express routing quality, cost, privacy, and capability intent at a higher level than raw
`OpenRouterProviderRouting` flags. Multiple presets may be combined.

`apply_presets()` compiles the preset list into `OpenRouterProviderRouting` flags and
clears `presets` from the resulting config. The driver calls this before serializing any
routing fields to the request wire format. Explicit `provider` fields always override
preset-derived values; when multiple presets target the same field, later ones win.

Presets are applied before capacity strategy, `apply_presets()` runs first, then
`apply_capacity_strategy()` runs on the result.

Preset definitions and their exact mappings live in
[`OpenRouterRoutingPreset` and its compiler](../../crates/drivers/openrouter/src/options.rs).
Price ceilings retain USD per million prompt/completion tokens, matching the
[OpenRouter routing contract](https://openrouter.ai/docs/guides/routing/provider-selection).
Catalog model pricing uses a different unit and must not be applied to routing ceilings.

`apply_presets()` returns `Err` for invalid inputs (e.g. negative `MaxPrice` values).
When `presets` is empty the driver skips calling `apply_presets()` entirely (clone-free fast path).

### Stateful Continuation Invariant

When a request to the OpenAI Responses API sets `previous_response_id`, the provider already holds the prior transcript server-side. The request must NOT also carry the full reconstructed transcript in `input`, that double-counts context and inflates prompt-cache keys.

Invariant: **a request with `previous_response_id` only carries delta items in `input`**: typically tool results (`function_call_output`) for the prior assistant turn plus any fresh user messages. Prior assistant messages, reasoning items, and the assistant's own function calls are dropped because they live in server-side state. `instructions` (system message) is sent separately and is exempt. Empty `input` is allowed.

`OpenResponsesProtocolChatDriver` enforces this by trimming `input` via `compute_delta_input_items` whenever `previous_response_id` is `Some(_)` (see `crates/provider/src/openresponses_protocol/mod.rs`).

### Provider-declared statefulness

Server-side continuation only works where the endpoint actually stores responses. OpenAI-compatible gateways that expose a stateless `/responses` shim, e.g. OpenRouter and Google Gemini's compat endpoint, *accept* `previous_response_id` but silently ignore it (`store: false`). Chaining against them drops the conversation from turn 2 onward (only the latest tool output reaches the model), so the agent loses the task and loops on exploration.

Statefulness is service semantics, not a hostname property. The provider
assembly explicitly enables it for OpenAI, Azure OpenAI, and Meta. Stateless
assemblies such as OpenRouter leave it disabled, so the driver drops
`previous_response_id` and replays the **full** transcript. Custom services opt
in only when their endpoint actually persists Responses state. No protocol
driver contains a host-based service dispatch.

## Automatic Retry for Transient Errors

LLM drivers implement automatic retry with exponential backoff for transient errors. This follows official SDK behavior from OpenAI and Anthropic.

### Retry Configuration

Default retry config (matches official SDKs):
- **max_retries**: 2
- **initial_backoff**: 1 second
- **max_backoff**: 60 seconds
- **backoff_multiplier**: 2.0
- **jitter_factor**: ±25%

### Transient Error Detection

The following HTTP status codes trigger automatic retry:
- `408` - Request Timeout
- `409` - Conflict
- `429` - Too Many Requests (Rate Limited)
- `5xx` - Server Errors (except 501 Not Implemented)

In-band stream errors inside an accepted response are retried only when all of
the following hold:

- no text, thinking, tool call, or completion output has been produced
- the structured provider code or HTTP status classifies the error as transient
- the bounded default retry budget has not been exhausted

Provider code is authoritative, followed by HTTP status; message matching is a
compatibility fallback for legacy drivers. `processing_error`, provider server
errors, ordinary rate limits, and transient HTTP statuses receive bounded retry.
Authentication, invalid-request, and exhausted billing/quota errors fail fast.
Once output exists, the runtime preserves it as a partial success instead of
replaying the generation and risking duplicate visible output.

A provider stream stall, the runtime's stream-liveness watchdog aborting a
stream that produced no tokens within its window, is treated the same way as an
in-band transient error: before any output it is classified transient and routed
through the same bounded retry path, re-issuing the identical request with no
artificial history; after output, or once the retry budget is exhausted, it
fails the turn. See `crates/engine/src/execution/reason.rs`.

### Rate Limit Header Support

Drivers parse provider-specific headers to determine retry timing:

**Standard Headers:**
- `retry-after` - Seconds to wait (integer or HTTP-date)
- `retry-after-ms` - Milliseconds to wait (used by OpenAI)

**Anthropic-specific:**
- `anthropic-ratelimit-requests-remaining`
- `anthropic-ratelimit-requests-reset`
- `anthropic-ratelimit-tokens-remaining`
- `anthropic-ratelimit-tokens-reset`

**OpenAI-specific:**
- `x-ratelimit-remaining-requests`
- `x-ratelimit-remaining-tokens`
- `x-ratelimit-reset-requests`
- `x-ratelimit-reset-tokens`

### Retry Metadata

On successful completion after retries, `LlmCompletionMetadata` includes retry info (attempts, total wait time, rate limit info). The `llm.generation` event also includes retry info. See `crates/provider/src/driver_registry.rs` for `RetryMetadata`.

### Implementation Details

1. **Retry-after cap**: Maximum wait time from `retry-after` headers is capped at 60 seconds
2. **Exponential backoff**: When no `retry-after` header, uses exponential backoff with jitter
3. **Rate limit type detection**: Distinguishes between request-based and token-based rate limits

## Context Compaction (OpenAI Responses API)

The `/v1/responses/compact` endpoint is a context-compression feature for the OpenAI Responses API. It reduces conversation context size when approaching the model's context window limit.

### How It Works

1. Send the current conversation window (the `input` items from `/v1/responses` calls)
2. The endpoint returns a compacted window where:
   - All prior **user messages** are kept verbatim
   - Prior assistant messages, tool calls/results, and encrypted reasoning are replaced by one encrypted **compaction item**
3. Use the returned `output` array as the `input` for the next `/v1/responses` call

The two request modes are intentionally disjoint:

- Compact by stateful handle: `previous_response_id` is present and `input` is
  empty.
- Compact by standalone transcript: `input` is present and
  `previous_response_id` is absent.

After either mode succeeds, the returned `output` is carried in
`ProviderOpaqueContext::OpenResponsesCompact` and reused, in its original
order, as the next Responses `input`. That retry omits `previous_response_id`
and bypasses transcript-delta trimming and tool-structure pruning. The context
enum supports lossless serialization for durable runtime checkpoints and uses
a payload-redacting `Debug` implementation.

If a later compact call replaces a restored checkpoint, the prior compact
output is converted item-for-item to compact input and placed before the raw
suffix. The compact request is standalone and omits `previous_response_id`;
this ordering rule is identical for proactive and reactive compaction.

### ChatDriver Compact Methods

The `ChatDriver` trait includes `supports_compact()` and `compact()` methods. See `crates/provider/src/driver_registry.rs` for `CompactRequest`, `CompactInputItem`, `CompactResponse`, and `CompactOutputItem` types.

The trait also exposes an optional effective context-window boundary. A driver
whose runtime model aliases or profiles are not represented in Everruns'
built-in provider/model table returns its authoritative context limit there.
Host proactive policy prefers that value and falls back to the built-in profile
only when the driver has no override. This keeps external drivers such as
`openai-codex` provider-neutral while preventing a guessed 128k fallback from
driving compaction policy.

The host records the raw source boundary before each proactive native call.
When a driver fails or returns a result without material token/byte reduction,
the host suppresses another call for that session/provider/model until estimated
input grows by at least 4,096 tokens and 5%. The source boundary remains the
monotonic identity, while an input-prefix fingerprint rejects watermarks from an
abandoned branch. Drivers do not need to implement this negative backoff
themselves. Attempt watermark I/O is advisory and fails open; an unavailable
external store must not prevent either compaction or the subsequent model call.
Material reduction means at least 5% and 32 measured tokens/bytes, providing
useful headroom instead of accepting any merely smaller response.

### Provider Support

| Provider | Compact Support |
|----------|----------------|
| OpenAI (Responses API) | Yes |
| OpenAI (Completions API) | No |
| Anthropic | No |
| Gemini | No |
| Bedrock | No |
| LlmSim | No |

## Key Resolution Contract (Fail-Closed)

This section is the single source of truth for **where provider credentials may
come from**. Read it before touching any provider assembly, `DriverConfig`
builder, provider store, or resolver. Violating it can silently fund tenant
execution from platform credentials, a cost-runaway and trust boundary
incident, not a cosmetic bug.

### The one rule

> **Driver code never reads provider *credentials* from the process
> environment.** No `std::env::var` for an API key, secret, or endpoint base URL
> in any `crates/drivers/openai`, `crates/drivers/anthropic`, `crates/drivers/gemini`,
> `crates/drivers/openrouter`, `crates/drivers/bedrock`, `crates/drivers/mai`, or the protocol drivers in
> `crates/core` (`openai_protocol.rs`, `openresponses_protocol.rs`). Credentials
> only ever arrive through a runtime provider assembly or a `DriverConfig`
> compatibility/catalog adapter.
>
> Scope: this bans reading *credentials* from env, not all environment access. A
> driver may still read a non-credential tuning knob from env (e.g.
> `OPENAI_IMAGE_TIMEOUT_SECS` in `crates/drivers/openai/src/images.rs`).

There are exactly **two** sanctioned ways credentials reach a driver. Every code
path must be one of them; there is no third option and no fallback between them.

| # | Path | Who uses it | Where credentials come from | Reads env? |
|---|------|-------------|-----------------------------|------------|
| 1 | **Server / tenant** | org-scoped agent + embedding execution | encrypted DB row, decrypted by the resolver | **Never** |
| 2 | **Standalone / dev / CLI** | `just start-dev`, examples, CLI tools, embedders | a `CredentialProvider` the caller injects | only via `EnvCredentialProvider`, constructed explicitly by that caller |

Path 2 has one ambient-identity variant: Bedrock's opt-in `default-credentials`
feature (`BedrockAuth::default_chain`, the facade's `Bedrock::default_chain`,
serve's `bedrock/<model-id>` route) hands credential resolution to the AWS SDK's
default chain, so a standalone host runs on its IAM role (AgentCore Runtime
execution role, ECS task role, instance profile). It is still caller-selected,
never a fallback, and `scripts/lib/check-provider-isolation.sh` keeps it out of
server, platform, and worker code and manifests: on a multi-tenant host it would
sign tenant calls with the platform's own AWS identity.

```
                         credentials for a driver
                                   │
              ┌────────────────────┴────────────────────┐
   org/tenant execution?                       standalone / dev / CLI?
              │                                           │
   resolve_provider_api_key()                  caller injects a CredentialProvider
   (encrypted DB, fail-closed)                 (EnvCredentialProvider for env vars)
              │                                           │
        DriverConfig ─────────────► runtime Provider ◄──────── Provider assembly
              (no env reads anywhere on either path)
```

### Path 1, Server-Side Tenant Path

All API key resolution for tenant/org-scoped execution flows through
`crates/server/src/services/provider_resolver.rs`. The contract is **fail-closed**:

1. If the provider has an encrypted key in the database and the encryption service
   is available, decrypt and return it. If decryption fails the call returns `Err`
   (not `None`).
2. If the provider has an encrypted key but the encryption service is unavailable,
   log a warning and return `None` (not `Err`).
3. If no database key is stored at all, return `None`.
4. Callers receiving `None` MUST NOT fall through to environment variable
   reads. A selected provider may still be assembled into a credential-gated
   driver so configuration commands can inspect the turn and repair settings.
   The gate rejects chat, model-listing, and compaction operations locally as
   an authentication/configuration error before any provider network I/O.

`ProviderStore::get_provider_config` has no default implementation. A custom
host must explicitly return its credential-bearing `ProviderConfig`, or return
`None` to declare that credentials are owned by a directly registered runtime
provider or are currently absent. This makes the credential boundary a
compile-time integration decision without putting secrets back into
`ModelSpec` or provider selection.

**Why**: With a platform-level `DEFAULT_*_API_KEY` present on the server host, an
implicit env fallback silently funds tenant execution from platform credentials.
Fail-closed prevents accidental cost-runaway under open signup.

### Path 2, Dev / CLI / Standalone Path

Env-based credential loading is an explicit, injected concern routed through a
single shared boundary in `crates/core/src/credential_provider.rs`:

- **`CredentialProvider`** (trait), the injectable source. Its one method,
  `resolve(&DriverDescriptor) -> Option<ProviderCredentials>`, returns the
  credentials for a driver, decoupled from where they came from. It takes the
  descriptor rather than a bare `DriverId` because only the driver knows which
  fields it needs and what they are called. Drivers and dev stores depend on
  this trait, not on the environment.
- **`ProviderCredentials`**: the driver's declared credential fields as a map,
  plus an optional `base_url`. Same shape an operator-entered form produces, so
  `document()` yields exactly what the server stores and multi-field drivers
  (Bedrock's four AWS fields, MAI's Entra block) are expressible.
- **`EnvCredentialProvider`**: the shared library implementation of the
  env-based source, and the **sanctioned pattern** for any caller that wants
  env-driven credentials. It is the only component in the workspace that pairs a
  driver's declared variable names with a real `std::env::var` lookup, which is
  what makes "keep it out of server wiring" sufficient to keep the environment
  out of server credential resolution.

**Names belong to the drivers, not to this resolver.** Each credential field
declares the variables its vendor's own SDK reads (`FormField::env`, plus
alternates the vendor also honors), and a driver declares its endpoint variable
as `DriverDescriptor::base_url_env`. Resolution walks that declaration, honoring
the same mutually-exclusive groups `CredentialFormSchema::validate` enforces, so
a half-populated OAuth block configures nothing rather than half-configuring a
provider.

The per-driver table is published in `docs/framework/credentials.md` and pinned
against the drivers' own declarations by
`crates/worker/tests/driver_env_declarations.rs`, so it cannot drift.

**A driver that declares no variable is never configured from the environment**,
however its id is spelled. That is the default the registry's built-in schema
gives; a driver opts in by naming what its vendor reads.

> This replaced a scheme that derived `<UPPERCASE_DRIVER_ID>_API_KEY` and
> `_BASE_URL` from the driver id centrally. That could express one key and one
> URL, so it silently resolved nothing for Bedrock and MAI — a gap its own test
> encoded as expected — and `openai_completions` needed its vendor's real names
> hardcoded in the resolver because the driver had nowhere to state them.

Standalone/dev/CLI entrypoints opt in by constructing `EnvCredentialProvider` and
passing it where credentials are needed, e.g.
`everruns_host::InMemoryProviderStore::from_credential_provider(&registry, &EnvCredentialProvider)`,
the in-memory dev store used by `just start-dev`. **The server path never
constructs an `EnvCredentialProvider`.**

Note: the `DEFAULT_*_API_KEY` env vars are a *separate* mechanism (Path 1 seed
materialization, below), not this provider. They are read at startup and written
into the DB, never read by a driver.

### Do / Don't

**Do**

- Attach credentials/auth and the endpoint to a runtime `Provider`; protocol
  driver constructors remain credential-free.
- In a standalone/dev/CLI entrypoint, construct `EnvCredentialProvider` (or any
  other `CredentialProvider`) and inject it, or call the driver crate's
  `from_env(id)`.
- Declare a driver's variables on its own credential schema and descriptor,
  using the names that driver's vendor SDK reads.

**Don't**

- ❌ Read a credential (`*_API_KEY`, secret, or endpoint base URL) from
  `std::env::var(...)` in any driver crate or protocol driver. (Non-credential
  knobs are fine.)
- ❌ Add a driver's variable names anywhere but that driver's own declaration.
  A central mapping keyed by driver id is what this design replaced.
- ❌ Construct `EnvCredentialProvider`, or call any `from_env`, anywhere
  reachable from org-scoped execution, or otherwise let a `None` from the
  resolver fall through to env.

**On `from_env`**

Driver-crate `from_env(id)` constructors are sanctioned, reversing an earlier
rule that banned them because "their name invites use on the server path". Two
things changed. They perform no env access of their own: each is a three-line
delegation to `provider_from_env`, which resolves through
`EnvCredentialProvider` and then builds the provider via the driver's own
registered factory — the same path an operator-entered credential takes. And
the invitation is now answered structurally rather than by convention: the
provider isolation guard (`scripts/lib/check-provider-isolation.sh`) fails the
build if server or platform code calls `from_env`, `provider_from_env`, or
constructs an `EnvCredentialProvider`.

The banned shape was a driver reading env *itself*. That remains banned.

### Adding a new driver

A new driver inherits the contract for free: implement only credential-taking
constructors, never an env read. To make it configurable from the environment
for dev/CLI use, declare the variables **its vendor's SDK reads** on its own
credential fields (`FormField::env`, `env_fallback` for alternates the vendor
also honors) and, if that vendor defines an endpoint variable,
`DriverDescriptor::base_url_env`. Then expose `descriptor()` and a `from_env(id)`
delegating to `provider_from_env`, and add the driver to the table in
`docs/framework/credentials.md` and to
`crates/worker/tests/driver_env_declarations.rs`.

Declaring nothing is a valid choice and the safe default: the driver is then
only ever configured explicitly. Never add the names to a central mapping —
that is the design this replaced.

### Single-Tenant / Dev: Startup Materialization

The resolver itself stays fail-closed everywhere; it never gains an env fallback.
To keep `DEFAULT_*_API_KEY` working for single-tenant self-hosted deploys and
`just start-dev` **without** re-opening the hot path, the server *materializes*
those env vars into the **default org's** seed provider rows at startup
(`seed::seed_default_provider_keys_from_env`). The key is encrypted and written
to the DB, so org-scoped execution resolves it through the same fail-closed DB
path as any user-configured key.

Rules:

- Only the **default org's** seed providers are filled, and only slots that have
  **no** key, a key configured via UI/API is never overwritten.
- Requires the encryption service (`SECRETS_ENCRYPTION_KEY`); if absent, the step
  is skipped with a warning.
- Gated by `materialize_env_provider_keys_allowed`:
  - `SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV` unset → defaults to `DeploymentGrade::is_dev()`
  - `true`/`1`/`yes` → enabled only when the deployment is dev or built-in auth
    cannot self-provision users into `DEFAULT_ORG_ID` (for example, full auth
    with signup disabled and no built-in OAuth providers, admin-only auth, or
    external auth)
  - anything else → disabled
- Multitenant/open-signup deployments leave this disabled, so platform-level keys
  are never seeded into an org that untrusted users can join and spend from. New
  orgs created at runtime (`seed_all` via the org CRUD path) do **not**
  materialize env keys.

### Invariant

> A tenant turn or tenant-triggered embedding that resolves without a database key
> must fail with a clear error, regardless of which environment variables are set
> on the host process.

This invariant is verified by the unit tests `resolve_provider_api_key_env_key_set_does_not_leak`
and `resolve_provider_credentials_env_key_set_does_not_leak` in
`crates/server/src/services/provider_resolver.rs`. The complementary rule, that
drivers never read env, is anchored by `EnvCredentialProvider` being the sole
env reader, with unit tests in `crates/core/src/credential_provider.rs`.

## Testing

1. **Unit Tests**: Each driver MUST have tests for error detection functions
2. **LlmSim**: Use `ProviderType::LlmSim` for integration tests and scripted demos without real API keys. It replays canned responses and runs no inference, so a demo on it shows the surrounding product working, never the model's behavior; public documentation must not present it as an offline mode of the product (see knowledge/framework/provider-extension.md). The production-safe driver lives in `everruns-llmsim` (`crates/drivers/llmsim/src/lib.rs`); core carries no simulation code, and product consumers depend on the simulator rather than test-support. `LlmSimConfig::scripted(...)` supports deterministic multi-turn scenarios with assistant text, tool calls, mixed turns, injected errors, and configurable exhaustion behavior. `everruns-test-support` uses the driver for its in-memory loop and keeps 0.17 simulator paths only as a 0.18 migration bridge.
3. **Error Detection Tests**: Cover all documented error patterns for each provider
4. **Parametrized Integration Tests**: Use `rstest` matrix in `crates/core/tests/`:
   - `llm_test_matrix/mod.rs`, shared `ProviderModelConfig` structs and `all_providers_registry()`
   - `agent_run_basic.rs`, basic completion + tool call, parameterized over all providers
   - `agent_run_with_thinking.rs`, extended thinking, parameterized over thinking-capable providers
   - Add new providers: one `const` in `llm_test_matrix` + one `#[case]` per test function
   - Tests skip gracefully when provider API key env var is not set
