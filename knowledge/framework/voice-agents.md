---
type: Specification
title: "Voice Agents"
description: "Accepted design for building and serving voice agents on Everruns: voice is a channel next to text channels, one voice loop in core shared by the Framework, serve and the platform server, and speech drivers as provider services."
tags:
  - everruns
  - framework
  - voice
  - serve
  - server
  - drivers
---

# Voice Agents

Status: Accepted (2026-10-09). Phase 1 is implemented: the contracts, the
OpenAI realtime driver and the core voice loop, and the platform server's voice
channel ([Voice Channels on the Platform Server](../operations/voice.md)).
Phase 2 is implemented for browser WebRTC: the Framework's `everruns::voice`
and serve's voice routes (public guide `docs/framework/voice.md`, example
`examples/serve/voice`). In-process audio, WebSocket audio, client secrets and
phase 3 are not built yet.
Scope: `everruns-contracts`, `everruns-drivers`, `everruns-core`, the
`everruns` facade, `everruns-serve`, and the platform server. Supersedes the
earlier session-only voice routes.

## Problem

Someone who wants a voice agent on Everruns today has no supported path.

- The platform server has a voice route (`crates/server/src/channels/voice/mod.rs`),
  but the `voice` feature flag is graded Dev, which
  `crates/contracts/src/runtime/feature_flag_grade.rs` defines as local
  development only.
- It works only with OpenAI Realtime over browser WebRTC. It waits for the
  whole answer before speaking: it polls the event store every 250 ms and gives
  up after 60 s (`wait_for_voice_answer`). Barge-in is off
  (`interrupt_response: false`).
- The Framework and `everruns-serve` have no voice at all. The only way to
  speak to a served agent is to run your own voice stack (LiveKit, Pipecat,
  Twilio) in front of `/v1/sessions/{id}/messages` and `/sse`.
- Speech is not a provider service. `ServiceKind::Realtime` exists and the
  OpenAI descriptor declares it, but there is no realtime, speech-to-text or
  text-to-speech driver trait. The server talks to OpenAI with hand-written
  HTTP and WebSocket code.

## Goals

1. A developer builds a voice agent the same way as any agent: an `Agent`
   with tools and capabilities. Voice is a channel the agent is exposed on,
   next to its text channels, so one agent can serve text and voice at once.
2. The same agent is reachable by voice from a browser, a mobile or desktop
   app, and a phone number, through the Framework, `everruns-serve`, and the
   platform server, with one implementation of the voice loop behind all three.
3. Speech providers are ordinary provider services: speech-to-speech
   (realtime), speech-to-text and text-to-speech, resolved and configured like
   chat and embeddings.
4. Voice turns feel live: first audio around 800 ms after the user stops
   talking for answers that need no tools, short spoken preambles while tools
   run, and interruption at any time.
5. The durable session stays the source of truth. The transcript records what
   the user said and what the user actually heard, so a dropped call resumes
   and a voice conversation can continue as text.
6. Tools, approvals, budgets and audit apply to voice exactly as to text.

## Non-goals

- Storing raw audio. Recording is a later, opt-in feature with consent and
  retention controls.
- Outbound AI calling in the first release. Under the FCC's 2024 TCPA ruling,
  AI voices count as artificial, so outbound calls need prior express
  consent. It ships later, behind its own flag.
- Phone calls (SIP, Twilio). They are a follow-up design that builds on
  this one; see "Follow-up: phone" below.
- Video and screen sharing.
- Our own speech models or our own WebRTC media server. Media goes to the
  speech provider or the telephony provider. When Everruns does carry audio,
  it only relays frames over WebSocket.
- A new crate. Everything below lands in existing crates behind features, per
  the crate-reduction policy.

## How a voice agent works

A voice agent is an ordinary Everruns agent exposed on a **voice channel**.
The channel owns a **voice front**.
The front listens, decides when the user has finished, and speaks. The agent
does the thinking, through the normal turn path (`start_turn`), with its own
model, tools and memory. A component in core, the **voice loop**, connects the
two.

```
 caller audio ──► transport ──► voice front ──transcript──► voice loop ──send/steer──► Session (agent turn)
                                    ▲                           │  ▲                         │
                                    └────────── speak ──────────┘  └── output deltas, phases ┘
```

### Three front modes

| Mode | What listens and speaks | What thinks | When to use |
|---|---|---|---|
| `delegated` (default) | Speech-to-speech model (OpenAI GPT-Live or Realtime, Gemini Live, xAI) | The Everruns agent, any model | Most agents. Natural voice and turn-taking from the speech model, with the full agent behind it |
| `cascaded` | Streaming speech-to-text plus streaming text-to-speech (OpenAI transcription and speech models; other vendors later) | The Everruns agent | Vendor choice, cost control, a specific voice, languages the speech models handle poorly |
| `native` | Speech-to-speech model | The same speech model, calling Everruns tools | Simple, latency-critical agents where the speech model is smart enough |

`delegated` is the default because it keeps Everruns' main promise: the agent
you built and tested in text is the agent that answers the phone. OpenAI's
GPT-Live "client delegation" mode is built for exactly this. The live model
handles speech and turn-taking, and the application runs any backend and
returns spoken text (`session.commentary.append`) or background context
(`session.thinking.append`). On Realtime, Gemini Live and xAI the loop gets the
same behaviour by turning off automatic responses and sending the agent's text
as out-of-band, audio-only responses, which `voice.rs` already does today.

In `native` mode the speech model holds the conversation. The loop puts the
agent's tools on the speech session, runs every function call through the
normal tool path (permissions, approvals, audit), and records the transcript.
Provider-executed MCP tools stay off unless the agent opts in, because they
skip Everruns policy.

### Voice is a channel

Voice is a channel type, like Slack, AG-UI or a public chat page. The same
agent can be exposed on a text channel and a voice channel at once, and each
channel carries its own settings, entry-point rules and budgets.

The voice channel's config holds everything about talking:

- mode (`delegated`, `cascaded`, `native`) and speech provider and model,
- voice, language, greeting,
- turn-taking (VAD type and eagerness), filler delay, interruption policy
  (`steer` or `cancel`),
- transports it accepts (browser WebRTC, WebSocket audio; phone later),
- who may call (org members, API keys, or anonymous with rate limits) and its
  budget.

What this gives:

- **One agent, many surfaces.** A support agent can answer on a web chat
  widget and a voice line with the same instructions, tools and memory. Two
  voice channels on one agent can sound different (a calm English line and a
  Spanish line) without forking the agent.
- **Entry-point rules stay where they belong.** Auth, rate limits and budgets
  are per channel already; voice minutes are expensive, so a voice line gets
  its own budget instead of sharing the text one.
- **Text and voice share a conversation when wanted.** A call can open a new
  session or attach to an existing one (the session binding channels already
  use), so a user can start in chat and continue by voice, or the other way
  round. Voice turns and text turns land in the same transcript.
- **Phone is just another transport.** The phone follow-up adds SIP and Twilio
  transports to the same channel type, not a new concept.

### The voice loop

The loop lives in `everruns-core` (`host::voice`, feature `voice`), so the
facade, serve and the server share it. It owns:

- **Input.** On end of turn (from the front's turn detection: server or
  semantic VAD, or a transcription model's end-of-turn events), it posts the
  final transcript as an `input.message` with `metadata.source = "voice"`. If a
  turn is already running, it steers it (`Session::send` already does
  start-or-steer), so "actually, make it Tuesday" lands in the current turn.
- **Output.** It subscribes to the turn's events instead of polling. It cuts
  `output.message.delta` text into sentence-sized pieces and speaks them as
  they arrive. `Commentary`-phase text (preambles such as "Let me check that")
  is spoken as soon as it is complete. Tool starts can trigger a short filler
  line from the voice channel's config when no commentary arrives within a
  configurable delay (default 1.5 s).
- **Interruption.** When the user starts talking, the front stops playback.
  The loop records what was heard, from the playout position (WebRTC and SIP
  report it, Twilio sends `mark` echoes, and TTS vendors return word
  timestamps). It then either lets the turn continue silently and steers it
  with the new utterance, or cancels it (`interruption: steer | cancel`,
  default `steer`).
- **Heard transcript.** When a spoken answer is cut off, the canonical
  `output.message.completed` keeps the full text, and a `voice.output.heard`
  event records the heard prefix and `interrupted: true`. Replay uses the
  heard text for the following turns, so the model does not believe it said
  things the user never heard.
- **Long tools.** The agent's turn keeps running while the call is live. When
  a tool outlasts the filler, the loop speaks the agent's progress
  (`tool.progress` text when present). Gemini's non-blocking function calls
  and GPT-Live's background thinking are used where available.
- **Lifecycle.** It owns call start and end, provider reconnects, and session
  resumption across provider limits (OpenAI 60 min, Gemini about 10 min per
  connection and 15 min per audio session, Nova Sonic 8 min). It reconnects
  with the durable transcript as context.
- **Metrics.** It records per-stage latency (end of turn, transcript final,
  agent first token, first audio, playout) and usage (audio minutes, audio
  tokens) as events and listener metrics.

The loop never touches raw audio when the provider owns the media (WebRTC or
SIP straight to the provider, with a sideband WebSocket). It relays audio
frames only for transports that hand audio to Everruns (the WebSocket audio
route now, phone relays later). It resamples between PCM16 rates and G.711
with a small resampler in core.

## Providers and drivers

### Contracts (`everruns-contracts`)

Following the embeddings and decisions pattern (`EmbeddingsDriver`,
`DecisionDriver`, factories on `DriverDescriptor`, fields on `RuntimeProvider`,
`create_*_driver` on `DriverRegistry`):

- `ServiceKind` gains `SpeechToText` and `TextToSpeech`. `Realtime` already
  exists and becomes backed by a real trait.
- `RealtimeDriver`: opens a speech-to-speech session and returns a
  bidirectional event stream. One neutral event set covers OpenAI Realtime,
  GPT-Live, Gemini Live, xAI and Nova Sonic: audio in and out, input and output
  transcript deltas, speech started and stopped, function calls, response
  done, interruption, usage, and provider limits (`GoAway`). It also covers
  provider-owned media: `accept_webrtc(sdp)`, `client_secret()`,
  `attach(call_id)`, and SIP `accept`, `reject`, `refer`, `hangup`.
- `SpeechToTextDriver`: streaming audio frames in, partial and final
  transcripts plus end-of-turn events out.
- `TextToSpeechDriver`: streaming text in, keyed by a context id with flush
  and cancel; audio chunks and word timestamps out.
- `AudioFormat`: PCM16 mono at 8, 16 or 24 kHz, G.711 μ-law and A-law.
- WebSocket client helpers (pinned DNS, dial, read) are pulled out of the
  Responses WebSocket transport
  (`openresponses_protocol/websocket_transport.rs`) behind the same feature,
  so speech drivers reuse them.

### Drivers (`everruns-drivers`)

Each vendor is a feature-gated module, as the drivers README requires.

| Vendor | Services | Notes |
|---|---|---|
| OpenAI | realtime (Realtime 2.x, GPT-Live), speech-to-text (realtime transcription), text-to-speech | **In scope.** The only vendor this design builds. GPT-Live client delegation is the reference `delegated` front |
| llmsim | realtime, speech-to-text, text-to-speech | **In scope.** Scripted transcripts and silent audio frames, so every voice test runs offline |
| xAI | realtime | Later, on demand. Near-copy of the OpenAI Realtime protocol, reuses that code with a different base URL |
| Google Gemini | realtime (Gemini Live) | Later, on demand. WebSocket only, session resumption handles |
| Amazon Bedrock | realtime (Nova Sonic) | Later, on demand. Bidirectional stream with SigV4 |
| Deepgram, ElevenLabs, Cartesia, AssemblyAI | speech-to-text and/or text-to-speech | Later, on demand, for `cascaded` mode only |

OpenAI alone covers all three modes: Realtime or GPT-Live for `delegated` and
`native`, and its transcription and speech models for `cascaded`. Other
vendors add choice (cost per minute, voice catalogue, languages, telephony
quality), not capability, so they wait until a user asks for one. The traits
are vendor-neutral so adding one is a driver module, not a redesign.

Model profiles already carry audio modalities (`profiles/realtime.rs`). Speech
models get model profiles for voices, formats, languages and per-minute or
per-token cost.

## Framework (`everruns` facade, feature `voice`)

The Framework has no channel registry, so a voice channel is a value the
application builds and points at a session. The agent stays unchanged.
Source: `crates/everruns/src/voice.rs`.

```rust
let voice = VoiceChannel::delegated(OpenAI::from_env()?.realtime())
    .voice("marin")
    .greeting("Hi, this is Bright Smile, how can I help?")
    .interruption(Interruption::Steer);

let session = engine.create(agent);
session.send("I need to move my Tuesday appointment").await?; // text

// Provider-owned WebRTC for a browser: SDP offer in, answer out.
let call = voice.accept_webrtc(&session, &offer_sdp).await?;
let answer_sdp = call.answer_sdp();
let mut events = call.events(); // transcripts, heard text, interruptions
call.end().await?;
```

- The call runs `everruns_core::voice::VoiceLoop` against the session: an
  utterance is `Session::send` with `metadata.source = "voice"` (start or
  steer), the `cancel` policy cancels the turn the last utterance started, and
  session output events feed the loop.
- `Realtime::simulated()` with `SimulatedCall` runs whole calls in unit
  tests, offline.
- `VoiceChannel::with_config` takes the platform's `VoiceChannelConfig`, so
  settings move between Framework and platform unchanged.
- Not built yet: in-process audio (`connect` with PCM frames), `say(text)`,
  and the cascaded and native modes.

## serve (`everruns-serve`, feature `voice`)

Built like the AG-UI route, not as a `#[channel]`: every top-level agent gets
a voice endpoint, and the optional `[voice]` section of `serve.toml` (a
`VoiceChannelConfig`) sets how calls sound for the whole app. This keeps
"voice is a channel" (one agent, text and voice at once) without a new macro
shape; per-agent settings can come later. Source: `crates/serve/src/voice.rs`;
wire contract in `crates/serve/docs/wire-api.md`.

| Route | Purpose |
|---|---|
| `GET /v1/channels/{agent}/voice` | Test page with a call button, so `cargo run` gives a working talk-to-it page (replaces the planned `voice.js` asset) |
| `POST /v1/channels/{agent}/voice/calls` | Browser WebRTC. Offer SDP in (plus an optional `session_id` to continue a conversation), answer SDP out. Media goes straight to the provider |
| `POST /v1/channels/{agent}/voice/calls/{call_id}/end` | End a call |

- Speech is OpenAI with `OPENAI_API_KEY`; `dev` and `eval` fall back to the
  simulator like models do, `start` refuses calls without a key.
- Running calls are held in-process by provider call id; the manifest and
  agent card list the routes.
- Not built yet: client secrets for direct dialing, WebSocket audio, and
  AgentCore/celld hosting of voice.

## Platform server

- A new `voice` value in `ChannelType` (`domains/agent_channels/record/mod.rs`) with a
  config struct next to the AG-UI and public chat ones. It is created and
  edited on the agent's Channels tab like other channels, with a "Talk to
  this channel" test button.
- Built: the call route sits under the agent's channel
  (`/v1/agents/{agent_id}/channels/{channel_id}/voice/calls`) next to the
  session route and the end route, authorized as org requests. Per-channel
  caller auth (anonymous lines, channel API keys, rate limits, budgets), the
  client-secret route and WebSocket audio are not built yet.
- `channels/voice/mod.rs` is rebuilt on the core voice loop and the new
  `RealtimeDriver`. That removes the hand-written OpenAI calls, the 250 ms
  polling and the 60 s cap, and turns on streamed speech and barge-in.
- The chat composer's microphone stays: in session chat it starts a call on
  that same session through the agent's voice channel, so a conversation
  moves between typing and talking. With no voice channel on the agent the
  microphone falls back to default voice settings. The old client-secret and
  attach routes are removed.
- `resolve_service` already resolves `Realtime` per org with a binding and an
  org default. It extends to `SpeechToText` and `TextToSpeech`, so an org
  picks its speech provider in Settings > Providers (OpenAI at first), and a
  voice channel can bind a specific one.
- The `voice` flag moves from Dev to Adoption once phase 1 ships, and to Prod
  after it has run on Adoption.
- In SaaS, metering counts audio minutes and tokens per speech service, and the
  spend cap ends a live call with a spoken notice rather than cutting it
  silently (owned by `saas/knowledge/llm/billing.md`).

## Security and privacy

- Provider keys stay server-side. Browsers get short-lived, single-call client
  secrets. `OpenAI-Safety-Identifier` keeps being set server-side.
- Tools run under the session owner through the normal permission resolver.
  Approval-gated tools get a spoken confirmation ("I'll cancel the 3 pm
  booking, okay?"), and the spoken yes or no answers the pending approval.
- No raw audio is stored. Transcripts are treated like chat messages for
  retention, export and audit.
- Each call announces up front that the caller is talking to an AI, through
  the greeting, which the agent can word but not turn off for phone channels.

## Testing

- llmsim speech drivers make the loop deterministic: scripted utterances,
  end-of-turn timing, interruptions at set playout positions, and provider
  disconnects.
- Core tests cover steering on interrupt, heard-prefix recording, sentence
  chunking, filler timing, reconnect with transcript, and tool approvals by
  voice.
- Live smoke tests per vendor follow the existing live-test gating (run only
  when the vendor's code changes, or by manual dispatch).
- A Mira study measures voice-to-first-audio latency per stage and per mode
  on a fixed script, so regressions show up as numbers.

## Phases

1. **Loop and OpenAI.** Contracts traits and events, the core voice loop,
   OpenAI realtime (Realtime 2.x and GPT-Live) and llmsim drivers. Rebuild the
   server's `voice.rs` on them (streamed speech, barge-in, no polling or cap),
   add the `voice` channel type with its UI and test button, then promote the
   flag to Adoption.
2. **Framework and serve.** The facade `voice` feature and `VoiceChannel`,
   serve voice routes over WebRTC with a test page, `examples/serve/voice`,
   and the public guide "Build a voice agent". Built; client secrets and
   WebSocket audio wait for a user who needs them.
3. **Cascaded and native on OpenAI.** Speech-to-text and text-to-speech traits
   with the OpenAI transcription and speech drivers, and native mode with
   Everruns tools on the realtime session.

Other vendors come when someone asks for one (see Drivers).

## Follow-up: phone

Not part of this design. The expected shape, for when it is picked up:

- Phone is a new transport on the existing voice channel: a phone number or
  SIP address in the channel config. Inbound calls create or resume a session
  per caller; caller ID is metadata only, never authorization.
- OpenAI SIP first: the `realtime.call.incoming` webhook (signature checked)
  accepts the call and attaches the same sideband, so no audio passes through
  Everruns. Twilio Media Streams second, through the WebSocket audio relay.
- In serve, the phone transport's secrets go in the manifest.
- Outbound calling behind its own flag, with a consent record.

## Decisions

- Default mode is `delegated` (user, 2026-10-09).
- OpenAI is the only speech vendor to build now; others on demand (user,
  2026-10-09).
- Phone is a follow-up (user, 2026-10-09).
- Voice is a channel, so one agent can be exposed over text and voice at
  once (user, 2026-10-09).

## Open questions

- Whether `interruption: steer` should also cut tool calls that have not
  started yet.

## Sources

- OpenAI Realtime: WebRTC, SIP, server controls (sideband), VAD,
  conversations. https://developers.openai.com/api/docs/guides/realtime-webrtc,
  https://developers.openai.com/api/docs/guides/realtime-sip,
  https://developers.openai.com/api/docs/guides/realtime-server-controls
- OpenAI GPT-Live delegation:
  https://developers.openai.com/api/docs/guides/live-delegation
- Gemini Live sessions: https://ai.google.dev/gemini-api/docs/live-session
- Twilio Media Streams messages:
  https://www.twilio.com/docs/voice/media-streams/websocket-messages
- Deepgram Flux: https://developers.deepgram.com/docs/flux/quickstart
- LiveKit turn handling: https://docs.livekit.io/agents/build/turns/
- FCC on AI voices under the TCPA:
  https://www.cooley.com/news/insight/2024/2024-02-15-fcc-ai-generated-robocalls-illegal-under-the-tcpa
