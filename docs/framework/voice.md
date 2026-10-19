---
title: Build a Voice Agent
description: Let people talk to a Framework agent and hear its answers, in your own Rust host or as a serve app, with the same agent, tools and transcript as text.
---

A voice agent is an ordinary agent with a **voice channel** in front of it. The
channel listens to the caller, turns each finished utterance into a user
message, and speaks the agent's answer as it streams. The agent itself does
not change: the same session can take typed messages and calls, and both land
in one transcript.

> **Status:** experimental. `everruns::voice` carries no compatibility promise
> yet, and the speech provider is OpenAI.

## Two modes

Your agent, with its own model and tools, writes every answer in both modes.
They differ in what listens and speaks, and in where the audio goes.

| Mode | Listens and speaks | Audio path | Use it for |
|---|---|---|---|
| **Delegated** (default) | A speech-to-speech model, `gpt-realtime-2` | Browser to OpenAI over WebRTC; never through your server | Browser calls with the lowest latency |
| **Cascaded** | A speech-to-text model (`gpt-4o-transcribe`) and a text-to-speech model (`gpt-4o-mini-tts`) | Through your process as PCM | Native and mobile apps, your own transport, picking each model separately |

## How a delegated call works

1. The browser sends a WebRTC offer with its microphone to your server. Your
   server places the call with OpenAI and returns the SDP answer, so the API
   key never reaches the browser. Audio then flows directly between the
   browser and OpenAI.
2. When the caller stops talking, the transcript becomes a user message with
   `metadata.source = "voice"`. If a turn is already running, the message
   steers it, as a typed message would.
3. The answer is spoken sentence by sentence while it streams. A filler line
   ("One moment.") covers a turn that stays silent for a while.
4. When the caller talks over the answer, speech stops at once. With
   `Interruption::Cancel` the running turn is cancelled too.

This is the same voice loop the Everruns server runs for its
[voice channels](/features/voice/), so a call behaves the same in your host,
in serve, and on the platform.

## In your own host

```bash
cargo add everruns --features voice,openai
```

```rust
# async fn demo(session: everruns::Session, offer_sdp: String) -> Result<(), Box<dyn std::error::Error>> {
use everruns::providers::openai::OpenAI;
use everruns::voice::{Interruption, VoiceChannel};

let voice = VoiceChannel::delegated(OpenAI::from_env()?.realtime())
    .voice("marin")
    .greeting("Hi, you are talking to an AI assistant. How can I help?")
    .speaking_style("Warm and unhurried.")
    .interruption(Interruption::Steer);

// `offer_sdp` is the browser's WebRTC offer, posted to your route.
let call = voice.accept_webrtc(&session, &offer_sdp).await?;
let answer_sdp = call.answer_sdp().to_string(); // send back to the browser

// Follow the call: what the caller said, what was spoken, interruptions.
let mut events = call.events();

// Hang up from the server, or wait for the caller to.
let summary = call.end().await?;
# let _ = (answer_sdp, events.try_recv(), summary);
# Ok(())
# }
```

`VoiceCall` keeps the call running until you end it, the caller hangs up, or
the value is dropped. Keep it somewhere your hang-up route can find it, keyed by
`call.call_id()`. Use `accept_webrtc_as` to pass OpenAI a stable,
privacy-preserving end-user id for abuse monitoring.

Write the agent's instructions for speech: short sentences, no markdown, lists
or links, numbers said the way a person says them. Delivery (pace, tone)
belongs in `speaking_style`; business rules stay in the agent.

### Cascaded calls

A cascaded channel takes audio through your process: 16-bit little-endian
PCM, mono, 24 kHz, in both directions. You connect it to the caller over any
transport, such as a WebSocket or a native app's audio stream.

```rust
# fn play(_pcm: Vec<u8>) {}
# fn drop_unplayed() {}
# async fn demo(session: everruns::Session, frame: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
use everruns::providers::openai::OpenAI;
use everruns::voice::{AudioOut, VoiceChannel};

let voice = VoiceChannel::cascaded(OpenAI::from_env()?.speech())
    .speech_model("gpt-4o-mini-tts")
    .voice("marin")
    .greeting("Hi, you are talking to an AI assistant. How can I help?");

let (call, mut audio) = voice.accept_audio(&session).await?;

// Caller audio in: forward microphone frames as they arrive.
audio.input.send(frame).await?;

// Speech out, already paced at playback speed.
while let Some(out) = audio.output.recv().await {
    match out {
        AudioOut::Audio(pcm) => play(pcm), // after what came before
        AudioOut::Clear => drop_unplayed(), // the caller talked over it
    }
}
# drop(call);
# Ok(())
# }
```

Dropping `audio.input` hangs up, as does `call.end()`. Each spoken sentence is
synthesized while the previous one plays, and when the caller talks over the
answer you get `AudioOut::Clear` at once. The call's events and summary work
as in delegated mode.

### Settings

| Builder | Default | What it does |
|---|---|---|
| `model` | `gpt-realtime-2` | Speech-to-speech model of a delegated channel. |
| `transcription_model` | `gpt-4o-transcribe` | Speech-to-text model for the caller, in both modes. |
| `speech_model` | `gpt-4o-mini-tts` | Text-to-speech model of a cascaded channel. |
| `voice` | `marin` | Provider voice. |
| `language` | none | Language hint for transcribing the caller, such as `en`. |
| `greeting` | none | Spoken when the call connects. Say that the caller is talking to an AI. |
| `turn_detection` | `ServerVad` | `ServerVad` ends a turn on a pause; `SemanticVad` waits for a finished thought. |
| `interruption` | `Steer` | Talking over the answer: `Steer` keeps the turn running, `Cancel` stops it. |
| `filler` | `One moment.` | Line spoken while the agent works. |
| `filler_after` | 1.5 s | Silence before the filler. Zero turns fillers off. |
| `speaking_style` | none | Delivery instructions for the speech model. |

`VoiceChannel::with_config` takes a whole `VoiceChannelConfig`, the same type
a platform voice channel stores, so settings move between the two unchanged.

### Test without a microphone

`Realtime::simulated()` places calls on an offline simulator. Play the caller
with `SimulatedCall`:

```rust
# async fn demo(session: everruns::Session) -> Result<(), Box<dyn std::error::Error>> {
use everruns::voice::{Realtime, SimulatedCall, VoiceChannel};

let call = VoiceChannel::delegated(Realtime::simulated())
    .greeting("Hello!")
    .accept_webrtc(&session, "v=0")
    .await?;
let caller = SimulatedCall::find(call.call_id()).unwrap();
caller.say("What is the weather?");
let spoken = caller.wait_spoken(2, std::time::Duration::from_secs(5)).await;
assert_eq!(spoken[0], "Hello!");
# Ok(())
# }
```

Pair it with [`Model::simulated`](/framework/testing-and-simulation/) and a
whole call runs in a unit test. For cascaded channels, `Speech::simulated()`
treats audio as text: each input frame is one utterance, and each output frame
is the UTF-8 text that was spoken.

## As a serve app

With the `voice` feature, every top-level agent of a
[serve](/framework/serve/) app takes calls:

```bash
cargo add everruns-serve --features voice
```

```toml
# serve.toml: how calls sound. Every field is optional.
[voice]
voice = "marin"
greeting = "Hi, you are talking to an AI assistant. How can I help?"
speaking_style = "Warm and unhurried."
```

Run `OPENAI_API_KEY=… cargo run -- dev` and open
`http://127.0.0.1:3000/v1/channels/{agent}/voice` for a test page with a
**Start call** button. Your own page uses two routes:

| Method | Route | Use |
|---|---|---|
| `POST` | `/v1/channels/{agent}/voice/calls` | Place a call: `{ "sdp": "…", "session_id"?: "…", "conversation"?: "…" }`. Answers `session_id`, `call_id` and `answer_sdp`. |
| `POST` | `/v1/channels/{agent}/voice/calls/{call_id}/end` | Hang up. Answers what the call did. |

```js
const pc = new RTCPeerConnection();
const mic = await navigator.mediaDevices.getUserMedia({ audio: true });
pc.addTrack(mic.getTracks()[0]);
const audio = new Audio();
audio.autoplay = true;
pc.ontrack = (event) => (audio.srcObject = event.streams[0]);
pc.createDataChannel("oai-events");

const offer = await pc.createOffer();
await pc.setLocalDescription(offer);
const response = await fetch("/v1/channels/concierge/voice/calls", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ sdp: offer.sdp }),
});
const { session_id, call_id, answer_sdp } = await response.json();
await pc.setRemoteDescription({ type: "answer", sdp: answer_sdp });
```

Each call lands in a session of the agent's voice channel:

- With `conversation`, your key for the caller (a user id, a phone number),
  every call with that key continues the same session.
- With `session_id`, the call continues that session, such as a chat that
  started typed.
- With neither, the call starts a new session.

Follow the transcript at `/v1/sessions/{session_id}/sse` like any session.

### Cascaded calls over a WebSocket

Set `mode = "cascaded"` and the app takes calls over a WebSocket instead:

```toml
[voice]
mode = "cascaded"
speech_model = "gpt-4o-mini-tts"
voice = "marin"
```

Connect to `ws://…/v1/channels/{agent}/voice/ws` (add `?conversation=…` or
`?session_id=…` to continue a conversation). Binary messages carry 16-bit PCM, mono, 24 kHz, both
ways. Text messages are JSON from serve:

| `type` | When |
|---|---|
| `call` | First message: `session_id`, `call_id`, `sample_rate`. |
| `clear` | The caller talked over the answer; drop audio not played yet. |
| `transcript` | A finished line: `role` (`user` or `assistant`) and `text`. |
| `interrupted` | Speech was cut off; `heard` is what was spoken. |
| `ended` | The call finished, with its counts. |

Closing the socket hangs up. The test page uses this transport when the app
is cascaded. An app takes one transport: the other route answers `400` and
names the right one.

Without `OPENAI_API_KEY`, `dev` and `eval` place calls on the offline
simulator (`model = "sim"` forces it, or `speech_model = "sim"` for a
cascaded app), and `start` refuses calls. Calls are
unauthenticated like every serve route, so put a public app behind your own
auth.

The [voice example](https://github.com/everruns/everruns/tree/main/examples/serve/voice)
is a hotel front desk you can call.

## On the platform

The Everruns server has [voice channels](/features/voice/) with the same
settings, behind the Voice feature flag. A Framework agent
[moved to the platform](/framework/moving-to-platform/) keeps its voice
settings by storing the same `VoiceChannelConfig` on a `voice` channel. The
server runs delegated channels only, so a cascaded agent switches to
delegated when it moves.

## Not yet

- Native mode, where the speech model answers by itself and calls the
  agent's tools.
- Phone numbers.
