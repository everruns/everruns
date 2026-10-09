---
type: Specification
title: "Voice Agents"
description: "Design for building and serving voice agents on Everruns: one voice loop in core shared by the Framework, serve and the platform server, speech drivers as provider services, and browser, WebSocket and phone transports."
tags:
  - everruns
  - framework
  - voice
  - serve
  - server
  - drivers
---

# Voice Agents

Status: Proposed (2026-10-09). Nothing below is implemented yet beyond what
"Today" lists.
Scope: `everruns-contracts`, `everruns-drivers`, `everruns-core`, the
`everruns` facade, `everruns-serve`, and the platform server. Supersedes the
transport and sideband parts of [Voice Sessions](../operations/voice.md) once
phase 1 lands.

## Problem

Someone who wants a voice agent on Everruns today has no supported path.

- The platform server has a voice route (`crates/server/src/api/voice.rs`),
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
   with tools and capabilities, plus a voice profile. Nothing else changes.
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
- Video and screen sharing.
- Our own speech models or our own WebRTC media server. Media goes to the
  speech provider or the telephony provider. When Everruns does carry audio,
  it only relays frames over WebSocket.
- A new crate. Everything below lands in existing crates behind features, per
  the crate-reduction policy.

## How a voice agent works

A voice agent is an ordinary Everruns agent with a **voice front** attached.
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
| `cascaded` | Streaming speech-to-text plus streaming text-to-speech (Deepgram, AssemblyAI, ElevenLabs, Cartesia, OpenAI) | The Everruns agent | Vendor choice, cost control, a specific voice, languages the speech models handle poorly |
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

### The voice loop

The loop lives in `everruns-core` (`host::voice`, feature `voice`), so the
facade, serve and the server share it. It owns:

- **Input.** On end of turn (from the front's turn detection: server or
  semantic VAD, or Deepgram Flux / AssemblyAI end-of-turn events), it posts the
  final transcript as an `input.message` with `metadata.source = "voice"`. If a
  turn is already running, it steers it (`Session::send` already does
  start-or-steer), so "actually, make it Tuesday" lands in the current turn.
- **Output.** It subscribes to the turn's events instead of polling. It cuts
  `output.message.delta` text into sentence-sized pieces and speaks them as
  they arrive. `Commentary`-phase text (preambles such as "Let me check that")
  is spoken as soon as it is complete. Tool starts can trigger a short filler
  line from the agent's voice profile when no commentary arrives within a
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
route and Twilio Media Streams). In those cases it converts between 8 kHz
μ-law and 16/24 kHz PCM16, a small resampler in core.

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
| OpenAI | realtime (Realtime 2.x, GPT-Live), speech-to-text (realtime transcription), text-to-speech | GPT-Live client delegation is the reference `delegated` front. WebRTC, WebSocket and SIP |
| xAI | realtime | Near-copy of the OpenAI Realtime protocol, reuses that code with a different base URL. No output truncation |
| Google Gemini | realtime (Gemini Live) | WebSocket only. Session resumption handles and context compression |
| Amazon Bedrock | realtime (Nova Sonic) | Bidirectional stream with SigV4. Later phase |
| Deepgram | speech-to-text (Flux with end of turn), text-to-speech (Aura) | New module |
| ElevenLabs | text-to-speech (multi-context WebSocket), speech-to-text (Scribe realtime) | New module |
| Cartesia | text-to-speech (Sonic, continuation contexts, timestamps) | New module |
| AssemblyAI | speech-to-text (Universal-Streaming) | New module, later phase |
| llmsim | realtime, speech-to-text, text-to-speech | Scripted transcripts and silent audio frames, so every voice test runs offline |

Model profiles already carry audio modalities (`profiles/realtime.rs`). Speech
models get profiles for voices, formats, languages and per-minute or
per-token cost.

## Framework (`everruns` facade, feature `voice`)

```rust
let agent = Agent::builder()
    .name("front-desk")
    .model(Model::new("gpt-6.1", openai.clone()))
    .instructions("You book appointments for a dental clinic.")
    .tool(book_slot)
    .voice(
        Voice::delegated(Realtime::openai("gpt-live-1").voice("marin"))
            .greeting("Hi, this is Bright Smile, how can I help?")
            .filler_after(Duration::from_millis(1500))
            .interruption(Interruption::Steer),
    )
    .build()?;

let session = engine.create(agent).await?;

// In-process audio: microphone and speaker frames from your app.
let call = session.voice().connect(AudioChannel::pcm16(24_000)).await?;

// Or provider-owned WebRTC for a browser: forward the SDP offer, return the answer.
let call = session.voice().accept_webrtc(offer_sdp).await?;
let answer_sdp = call.answer_sdp();

call.on_event(|e| /* transcripts, heard text, latency */);
call.end().await?;
```

- `Voice::cascaded(stt, tts)` and `Voice::native(realtime)` are the other two
  modes.
- A `VoiceCall` handle exposes events, `say(text)` for application-driven
  speech, `transfer`, `hang_up` and `end`. Calls are session resources,
  released on drop.
- Voice events reach `Engine` listeners like every other event, so the
  OpenTelemetry and Braintrust integrations trace voice turns with no extra
  work.
- A `coding-cli`-style example, `examples/voice-agent`, talks to an agent from
  the terminal through the local microphone. It also runs offline on llmsim.

## serve (`everruns-serve`, feature `voice`)

serve keeps its `/v1`-aligned shape. With the `voice` feature and an agent
that has a voice profile, `serve::start` adds:

| Route | Purpose |
|---|---|
| `POST /v1/sessions/{id}/voice/calls` | Browser WebRTC. Offer SDP in, answer SDP out, media goes straight to the provider |
| `POST /v1/sessions/{id}/voice/client-secret` | Short-lived provider token for clients that dial the provider directly |
| `GET /v1/sessions/{id}/voice/ws` | WebSocket audio for native and mobile apps and for `cascaded` mode: binary PCM16 frames both ways, JSON control messages (start, mark, clear, transcript, end) |
| `POST /v1/sessions/{id}/voice/{call_id}/end` | End a call |
| `POST /v1/channels/{name}/voice/twilio` + `GET .../twilio/stream` | Phone: TwiML that opens a bidirectional Media Stream, and the stream WebSocket |
| `POST /v1/channels/{name}/voice/sip` | Phone: the OpenAI `realtime.call.incoming` (or GPT-Live) webhook, signature checked, accepts the call and attaches the sideband |

- Phone channels are declared like other serve channels, with their secrets
  in the manifest (`#[connection]`, `Secret::named`), so a deploy knows which
  webhook URL and keys it needs.
- serve enables axum's `ws` feature only under `voice` (serve-agentcore
  already uses it for `/ws`).
- A static `voice.js` client in `serve::assets!()` connects a page to
  `/voice/calls` in a few lines, so `cargo run` gives a working talk-to-it page.
- AgentCore and celld hosting get voice where the host allows WebSocket.
  AgentCore's `/ws` carries the same audio protocol.

## Platform server

- `api/voice.rs` is rebuilt on the core voice loop and the new
  `RealtimeDriver`. That removes the hand-written OpenAI calls, the 250 ms
  polling and the 60 s cap, and turns on streamed speech and barge-in. The
  existing routes stay and gain `/voice/ws`.
- `resolve_service` already resolves `Realtime` per org with a binding and an
  org default. It extends to `SpeechToText` and `TextToSpeech`, so an org picks
  its speech vendors in Settings > Providers.
- Agents get a Voice section: mode, speech provider and model, voice,
  greeting, filler, interruption policy, and a "Talk to this agent" button.
  The profile is stored on the agent and versioned with it.
- A new `voice` value in `ChannelType` (`records/agent_channel.rs`) handles
  phone. Its config holds the transport (`openai_sip`, `twilio`), the number
  or SIP address, and the greeting. Inbound calls create or resume a session
  per caller. The caller ID becomes session metadata and is never trusted for
  authorization.
- Public chat pages (`public_chat`) can enable voice for anonymous visitors,
  with the same rate limits and budgets as text.
- The `voice` flag moves from Dev to Adoption once phase 1 ships, and to Prod
  after phone support has run on Adoption.
- In SaaS, metering counts audio minutes and tokens per speech service, and the
  spend cap ends a live call with a spoken notice rather than cutting it
  silently (owned by `saas/knowledge/llm/billing.md`).

## Security and privacy

- Provider keys stay server-side. Browsers get short-lived, single-call client
  secrets. `OpenAI-Safety-Identifier` keeps being set server-side.
- Tools run under the session owner through the normal permission resolver.
  Approval-gated tools get a spoken confirmation ("I'll cancel the 3 pm
  booking, okay?"), and the spoken yes or no answers the pending approval.
- Phone webhooks verify provider signatures (OpenAI webhook secret, Twilio
  request signature). Caller ID is metadata only.
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
   then promote the flag to Adoption.
2. **Framework and serve.** The facade `voice` feature, serve routes (WebRTC,
   client secret, WebSocket audio), `voice.js`, `examples/voice-agent`, and a
   public docs page, "Build a voice agent".
3. **Cascaded.** Speech-to-text and text-to-speech traits, plus OpenAI,
   Deepgram, ElevenLabs and Cartesia drivers. Provider settings for speech
   services in the server.
4. **Phone.** OpenAI SIP and Twilio Media Streams in serve and the server
   (`voice` channel type, agent Voice section, "Talk to this agent").
5. **More providers.** Gemini Live, xAI, Nova Sonic, AssemblyAI. Native mode
   hardening. Then outbound calling behind its own flag, with a consent
   record.

## Open questions

- Default mode: `delegated` (proposed) or `native`.
- First cascaded vendors: proposed Deepgram for speech-to-text and Cartesia or
  ElevenLabs for text-to-speech.
- Phone order: OpenAI SIP first (no audio through Everruns, smallest) or
  Twilio first (most customers already have Twilio numbers). Proposed: SIP
  first, Twilio in the same phase.
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
