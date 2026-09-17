---
title: Supported Providers
description: Every model provider driver Everruns ships today, the wire protocol each speaks, and which services it can power.
---

Everruns talks to model vendors through **drivers**. A driver owns one vendor's
wire protocol; a [`Provider`](/framework/models-and-providers/) pairs a driver
with an endpoint and a credential. The set below is what ships today — the
boundary is open, so a [custom driver](/framework/custom-providers/) is a
first-class peer of these, not a lesser one.

## Drivers

| Driver | Crate | Wire protocol | Services | Model discovery |
| --- | --- | --- | --- | --- |
| OpenAI | `everruns-openai` | OpenAI Responses | chat, embeddings, realtime | yes |
| OpenAI (Chat Completions) | `everruns-openai` | OpenAI Chat Completions | chat | yes |
| Azure OpenAI | `everruns-openai` | OpenAI Responses | chat | yes |
| Anthropic | `everruns-anthropic` | Anthropic Messages | chat | yes |
| Google Gemini | `everruns-gemini` | Gemini `generateContent` | chat | yes |
| AWS Bedrock | `everruns-bedrock` | Bedrock `ConverseStream` (SigV4) | chat | yes |
| OpenRouter | `everruns-openrouter` | OpenAI Responses-compatible | chat | yes |
| Microsoft MAI | `everruns-mai` | OpenAI Chat Completions (Azure AI Foundry) | chat | yes |
| Fireworks AI | `everruns-fireworks` | OpenAI Chat Completions-compatible | chat | yes |
| Meta Model API | `everruns-meta` | OpenAI Responses-compatible | chat | yes |
| LLM Simulator | `everruns-llmsim` | none — in-process test double | chat | no |

Every chat driver produces an incremental stream — server-sent events for the
HTTP protocols, `ConverseStream` for Bedrock — so token-by-token output works
everywhere, not just on one vendor. Tool calling and multi-turn tool results
work across all of them: each driver normalizes its vendor's shape into the same
typed events, which is why swapping a provider does not change application
code.

The environment variables each driver reads are in
[Credentials](/framework/credentials/).

## The simulator is not a provider

`everruns-llmsim` is listed above because it registers as a driver, but it runs
no inference and reaches no network. It replays canned responses so tests and
examples can assert on agent behavior without an API key. It is not a local
model and not a way to run Everruns without a vendor.

## Beyond chat

Most drivers implement chat only. Two capabilities go further, and both are
OpenAI-only today:

**Embeddings.** The OpenAI driver powers embedding models for knowledge-base
retrieval alongside its chat models.

**Realtime voice (WebRTC + WebSocket).** A realtime voice session is negotiated
by the platform server, not the Framework. The browser posts its SDP offer to
`POST /v1/sessions/{session_id}/voice/calls` and the server answers it; a
separate route mints a short-lived client secret from the vendor. The
organization's own API key is used only server-side and never reaches the
browser. The server then opens a WebSocket sideband
(`wss://…/realtime?call_id=…`) to drive the call and collect transcripts, which
land in the session as ordinary events — so a voice turn and a typed turn are
the same session, readable through the same history and event streams. This
needs the platform server; an embedded Framework process does not expose it.

## Interactive connect

OpenRouter declares an OAuth connect flow, so an operator can choose "Connect
with OpenRouter" instead of pasting a key. Every other driver takes a credential
directly, entered in Settings or supplied in code.

## Choosing one

Any OpenAI-compatible gateway that speaks Responses or Chat Completions can
usually be reached by pointing the matching driver's `base_url` at it, rather
than writing a driver. Write a [custom driver](/framework/custom-providers/)
when the vendor's protocol genuinely differs, or when it needs authentication
that a bearer token cannot express.
