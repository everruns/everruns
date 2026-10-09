---
type: Specification
title: "Voice Channels on the Platform Server"
description: "How the platform server runs voice calls: the voice channel type, call routes, the delegated voice loop against a session, leases, events and the Adoption flag."
tags:
  - everruns
  - operations
  - voice
---
# Voice Channels on the Platform Server

Status: Implemented (phase 1 of [Voice Agents](../framework/voice-agents.md)),
behind the `voice` feature flag, graded Adoption.
Public guide: `docs/features/voice.md`.

## Model

Voice is a channel type (`voice` in `records/agent_channel.rs`). Its config is
`VoiceChannelConfig` from `everruns_contracts::voice`, the same struct the
Framework and serve use, so validation and defaults live in one place.
Creating a voice channel, or importing one from an agent package, needs the
`voice` flag. A voice channel has no `auth` block and no inbound bindings: a
call is authorized as an ordinary org request (member session or org API key).

A call always belongs to one Everruns session. The session is the durable
source of truth; the speech provider session is ephemeral transport.

## Routes

`crates/server/src/api/voice/mod.rs` mounts three routes, all org-scoped and gated
on the flag:

- Call a channel: creates a session for the agent (tag `voice`) unless one is
  given, then places the call.
- Call on an existing session: uses the named channel, else the agent's first
  enabled voice channel, else defaults. A non-voice channel is a 400.
- End a call.

Browser WebRTC with server-side SDP exchange is the only transport. The
earlier client-secret and attach routes are removed: the server must hold the
provider call to drive the loop, and SDP proxying already gives it the call id.

## Call lifecycle

`api/voice/call.rs`:

1. Resolve the org's `Realtime` provider service through
   `ProviderResolverService::resolve_realtime` (optional `provider_id`
   binding), giving a `RealtimeDriver` and endpoint.
2. `accept_webrtc` with the channel's session settings and a server-derived
   safety identifier (never browser-supplied).
3. Record a leased resource (`voice_connection`, provider label `openai`, one
   hour) and emit `voice.session.started`.
4. Spawn the call task: attach to the provider call, subscribe to session
   events, and run `everruns_core::voice::VoiceLoop`. The session port turns a
   finished utterance into a user message (`metadata.source = "voice"`) and an
   interruption under the `cancel` policy into a session cancel. Agent output
   events become loop input.
5. The call ends on hang-up, the end route, the one-hour cap, or a provider
   failure; the lease is released and `voice.session.ended` or
   `voice.session.failed` is emitted.

Running calls are tracked in-process by cancellation token, so the end route
stops a call only on the server that runs it; the lease release and event
happen regardless. Multi-replica call routing is not solved yet.

Lease rows record no owner: lease owners are virtual users, and a call is
authorized through its session.

## Events

Lifecycle (`voice.session.*`), transcripts (`voice.input_transcript.*`,
`voice.output_transcript.*`) and `voice.output.interrupted` (heard and unspoken
text, policy). Answer latency and provider errors go to logs only. No raw
audio, SDP or provider payloads are stored.

## Testing

The llmsim provider declares a simulated realtime service
(`everruns_llmsim::realtime`), so integration tests in
`crates/server/tests/server_integration/api_integration_test/voice.rs` run a
whole call (greeting, utterance to message, streamed answer to speech, end)
without network.

## Not yet

- Cascaded and native modes (phase 3).
- WebSocket audio and phone transports.
- Per-channel caller auth (anonymous lines, channel API keys) and voice
  budgets.
