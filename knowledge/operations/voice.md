---
type: Specification
title: "Voice Sessions"
description: "Voice Sessions."
tags:
  - everruns
  - operations
---
# Voice Sessions

## Abstract

Voice Sessions let users talk to an Everruns session, an agent-backed session, or the singleton Platform Chat session through a low-latency Realtime API connection. The Everruns session remains the durable source of truth; the provider realtime session is an ephemeral transport and model state used while the microphone is connected.

The initial provider target is OpenAI `gpt-realtime-2`, using the Realtime API GA patterns documented in:

- https://developers.openai.com/api/docs/guides/realtime
- https://developers.openai.com/api/docs/guides/realtime-webrtc
- https://developers.openai.com/api/docs/guides/realtime-server-controls
- https://developers.openai.com/api/docs/models/gpt-realtime-2
- https://openai.com/index/advancing-voice-intelligence-with-new-models-in-the-api/

## Goals

- Add voice entry points for existing sessions, agent chat, and Platform Chat without creating a second conversation model.
- Keep provider API keys, tool execution, platform-management tools, and policy checks server-side.
- Persist useful transcript-level history in Everruns events and messages. Do not store raw microphone or assistant audio in V1.
- Support `gpt-realtime-2` reasoning, preambles, server-side tool calls, interruptions, and long-session context.
- Let the same chat UI render text turns and voice transcript turns in `/sessions/{id}/chat`, agent chat, and Platform Chat threads.

## Non-Goals

- SIP, telephony, call recording, or phone-number routing.
- Realtime translation and streaming-only transcription endpoints. Those use `gpt-realtime-translate` and `gpt-realtime-whisper` and should be specified separately.
- Offline audio upload or text-to-speech file generation.
- Running arbitrary OpenAI Realtime sessions detached from an Everruns session.

## Core Model

### Voice Connection

A Voice Connection is a short-lived session resource tied to one Everruns session.

Fields:

| Field | Description |
|-------|-------------|
| `id` | Prefixed public ID, `voice_conn_...` |
| `session_id` | Everruns session that owns the connection |
| `provider_type` | `openai` for V1 |
| `provider_id` | Optional configured LLM provider used for the OpenAI API key |
| `model` | Realtime model ID, default `gpt-realtime-2` |
| `voice` | Provider voice, default deployment-configured value |
| `reasoning_effort` | `minimal`, `low`, `medium`, `high`, or `xhigh`; default `low` |
| `transport` | `webrtc_proxy` or `webrtc_client_secret` |
| `provider_call_id` | Provider call ID from `/v1/realtime/calls`, when known |
| `status` | `starting`, `active`, `ended`, `failed` |
| `expires_at` | Lease/client-secret expiry timestamp used for cleanup and UI reconnect decisions |
| `started_at`, `ended_at` | Lifecycle timestamps |

Voice Connections should be registered as session resources and as leased resources. Cleanup closes sideband sockets, expires provider credentials, and marks stale active connections ended.

### Durable Transcript

Realtime audio is ephemeral. Everruns persists transcript text:

- User speech commits become `input.message` with `metadata.source = "voice"`.
- Assistant answers produced by the durable Everruns turn become `output.message.completed`; when Realtime is used as speech transport for that turn, provider output transcripts remain `voice.output_transcript.*` observability events and do not create duplicate canonical assistant messages.
- Assistant commentary/preambles may be emitted as `voice.output_transcript.delta` and optionally stored as non-canonical transcript metadata.
- Tool calls and results use the existing `tool.*` events so observability, audit logging, and UI tool cards stay consistent.

The persisted text transcript is the canonical replay input for future text or voice turns. Raw audio is not stored unless a future recording feature explicitly adds retention controls and consent.

## API

All endpoints are authenticated, org-scoped, and mounted under `/api/v1`.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/sessions/{session_id}/voice/client-secret` | Create a Voice Connection and mint an OpenAI Realtime client secret for direct browser WebRTC |
| POST | `/v1/sessions/{session_id}/voice/calls` | Preferred browser bootstrap: proxy SDP to OpenAI `/v1/realtime/calls`, capture the provider call ID, and return the SDP answer |
| POST | `/v1/sessions/{session_id}/voice/{voice_connection_id}/attach` | Attach a provider call ID after client-secret bootstrap so the server can open the sideband |
| POST | `/v1/sessions/{session_id}/voice/{voice_connection_id}/end` | End the Voice Connection and close provider sideband state |
| POST | `/v1/agents/{agent_id}/voice/sessions` | Create an Everruns session for an agent and return the same voice bootstrap payload |
| POST | `/v1/sessions/chat/voice` | Get or create Platform Chat, then return the same voice bootstrap payload |

### Voice Calls Request

`POST /v1/sessions/{session_id}/voice/calls` is the preferred browser
bootstrap path. It accepts browser SDP, proxies it to OpenAI
`/v1/realtime/calls`, captures the provider call ID from the response, opens the
server sideband, and returns the SDP answer.

```json
{
  "sdp": "v=0...",
  "model": "gpt-realtime-2",
  "voice": "marin",
  "reasoning_effort": "low",
  "locale": "en-US",
  "timezone": "America/Chicago"
}
```

Rules:

- `model` must resolve to an enabled OpenAI realtime model profile. V1 accepts only `gpt-realtime-2`.
- `reasoning_effort` defaults to `low`.
- `sdp` is required.
- Response `voice_connection.transport` is `webrtc_proxy`.
- The API key used to proxy SDP is never returned to the browser.
- The server sets `OpenAI-Safety-Identifier` when creating the provider session, using the same stable privacy-preserving user identifier strategy as other OpenAI calls.

### Voice Calls Response

```json
{
  "voice_connection": {
    "id": "voice_conn_01933b5a00007000800000000000001",
    "session_id": "session_01933b5a00007000800000000000002",
    "status": "active",
    "model": "gpt-realtime-2",
    "voice": "marin",
    "reasoning_effort": "low",
    "transport": "webrtc_proxy",
    "expires_at": "2026-05-07T20:15:00Z"
  },
  "answer_sdp": "v=0...",
  "client_secret": null
}
```

### Client Secret Request

`POST /v1/sessions/{session_id}/voice/client-secret` is the direct browser
bootstrap path. It does not accept SDP. The browser later connects to OpenAI
directly and must call `attach` with the provider call ID so the server can open
the sideband.

```json
{
  "model": "gpt-realtime-2",
  "voice": "marin",
  "reasoning_effort": "low",
  "locale": "en-US",
  "timezone": "America/Chicago"
}
```

Rules:

- `sdp` is not accepted.
- Response `voice_connection.transport` is `webrtc_client_secret`.
- `answer_sdp` is null.
- `client_secret` contains the provider ephemeral token shape returned by OpenAI.
- The standard provider API key used to mint the client secret is never returned to the browser.

## Provider Session Configuration

The server builds the provider session with:

- `type: "realtime"`
- `model: "gpt-realtime-2"`
- `audio.output.voice` from request or org default
- `reasoning.effort` from request or default `low`
- Instructions derived from the resolved harness, assigned agent, session hints, locale/timezone, and voice-specific prompt guardrails
- Tool definitions generated from the effective session capabilities

Voice-specific prompt additions:

- Use short preambles only when work is happening and silence would feel broken.
- Do not reveal private reasoning.
- For unclear audio, ask for a short clarification instead of guessing or calling tools.
- Confirm high-precision identifiers before account-specific lookup or write tools.
- Confirm before write actions, external effects, purchases, cancellations, payments, dangerous deletes, or agent/harness modifications.
- Use `commentary` for preambles/tool updates and `final_answer` for final user-facing speech when the provider exposes phases.

## Server Sideband

The backend opens a sideband WebSocket for each active Voice Connection when it knows `provider_call_id`. The sideband:

- Mirrors provider events into Everruns `voice.*`, `tool.*`, `input.message`, and `output.message.*` events.
- Handles provider tool calls by invoking the existing capability/tool execution path under the session owner's caller context.
- For the Platform Chat voice path, disables automatic Realtime responses and sends the durable Everruns final answer back over the sideband as an audio-only Realtime `response.create`.
- Applies the same permission, policy, audit, budget, and multitenancy checks as text turns.
- Can send provider `session.update` events when session state, tools, or prompt context changes.
- Closes when the client ends the call, the provider ends the call, auth expires, or the session is deleted/cancelled.

The browser data channel is for UI-level state only. It must not execute business tools, hold provider API keys, or bypass server authorization.

## Chat UI

The shared chat composer gains a microphone control when:

- `voice` feature flag is enabled,
- browser media APIs are available,
- the session has an OpenAI realtime-capable model/provider path,
- the caller can send messages to the session.

Surfaces:

- Session chat: `/sessions/{session_id}/chat` starts voice against that existing session.
- Agent chat: creating a voice chat from an agent uses `POST /v1/agents/{agent_id}/voice/sessions`, then navigates to the new session chat route.
- Platform Chat: `POST /v1/sessions/chat/voice` resolves the singleton Platform Chat session and returns the same voice bootstrap payload, showing the transcript in that session. Its only UI caller was the `/chat` page retired with EVE-855, so no first-party surface calls it today — `apps/ui/src/lib/api/voice.ts` still exports `startChatVoice` for it, unused.

UI states:

- Idle, connecting, listening, assistant speaking, tool running, interrupted, ended, failed.
- User transcripts stream into the existing transcript view as voice-marked user rows.
- Assistant transcript deltas render like normal streaming output, with preambles visually distinct but not stored as final answers unless finalized by the provider.
- Tool cards reuse existing chat tool renderers.
- Text input remains available during an active voice session; text turns sent during voice are forwarded over the Realtime data channel when possible, otherwise queued until the voice session ends.

## Interruption And Cancellation

User speech interruption should interrupt provider output without cancelling the Everruns session. `POST /v1/sessions/{session_id}/cancel` remains the hard stop for durable text turns. Ending a Voice Connection stops only the realtime audio connection and emits `voice.session.ended`.

If a tool call is already executing when voice ends, the server lets safe read-only calls finish and rejects or cancels pending write/dangerous calls unless the user already confirmed them.

## Security And Privacy

- Standard OpenAI API keys stay server-side.
- Ephemeral client secrets are scoped to one provider realtime session and short expiry.
- `OpenAI-Safety-Identifier` is set server-side and never accepts a browser-supplied value.
- Tool execution uses the normal session owner and permission resolver.
- Voice endpoints enforce the same org/session authorization as message creation.
- No raw audio storage in V1.
- Transcript events can contain sensitive spoken content and must be treated like chat messages for export, retention, audit, and observability controls.
- UI must make AI voice interaction clear before microphone capture starts.

## Observability

Voice Connections emit:

- lifecycle events for connection start/end/failure,
- transcript deltas/completions,
- provider call IDs in server logs only,
- model, reasoning effort, voice, transport, duration, and token usage in sanitized event metadata.

Do not persist provider client secrets, raw SDP bodies, or full provider sideband payloads in events or logs.

## Rollout

1. Add backend model/profile support for `gpt-realtime-2` behind `FEATURE_VOICE`.
2. Implement session voice bootstrap and sideband with proxy WebRTC transport first.
3. Add transcript persistence and chat UI microphone control for existing session chat.
4. Add agent shortcut and Platform Chat shortcut.
5. Add client-secret transport only after the attach flow reliably captures provider call IDs for sideband.
6. Add manual UI tests for browser permission denial, successful session voice, Platform Chat voice, tool-call voice, interruption, and transcript persistence.
