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
> yet, and the speech provider is OpenAI Realtime.

## How a call works

Calls run in **delegated** mode. A speech-to-speech model (`gpt-realtime-2` by
default) only listens and speaks; your agent, with its own model and tools,
writes every answer.

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
```

`VoiceCall` keeps the call running until you end it, the caller hangs up, or
the value is dropped. Keep it somewhere your hang-up route can find it, keyed by
`call.call_id()`. Use `accept_webrtc_as` to pass OpenAI a stable,
privacy-preserving end-user id for abuse monitoring.

Write the agent's instructions for speech: short sentences, no markdown, lists
or links, numbers said the way a person says them. Delivery (pace, tone)
belongs in `speaking_style`; business rules stay in the agent.

### Settings

| Builder | Default | What it does |
|---|---|---|
| `model` | `gpt-realtime-2` | Speech model that listens and speaks. |
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
use everruns::voice::{Realtime, SimulatedCall, VoiceChannel};

let call = VoiceChannel::delegated(Realtime::simulated())
    .greeting("Hello!")
    .accept_webrtc(&session, "v=0")
    .await?;
let caller = SimulatedCall::find(call.call_id()).unwrap();
caller.say("What is the weather?");
let spoken = caller.wait_spoken(2, std::time::Duration::from_secs(5)).await;
assert_eq!(spoken[0], "Hello!");
```

Pair it with [`Model::simulated`](/framework/testing-and-simulation/) and a
whole call runs in a unit test.

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
| `POST` | `/v1/channels/{agent}/voice/calls` | Place a call: `{ "sdp": "…", "session_id"?: "…" }`. Answers `session_id`, `call_id` and `answer_sdp`. |
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

A call without `session_id` starts a session tagged `voice`; pass one to keep
talking where a chat left off. Follow the transcript at
`/v1/sessions/{session_id}/sse` like any session.

Without `OPENAI_API_KEY`, `dev` and `eval` place calls on the offline
simulator (`model = "sim"` forces it), and `start` refuses calls. Calls are
unauthenticated like every serve route, so put a public app behind your own
auth.

The [voice example](https://github.com/everruns/everruns/tree/main/examples/serve/voice)
is a hotel front desk you can call.

## On the platform

The Everruns server has [voice channels](/features/voice/) with the same
settings, behind the Voice feature flag. A Framework agent
[moved to the platform](/framework/moving-to-platform/) keeps its voice
settings by storing the same `VoiceChannelConfig` on a `voice` channel.

## Not yet

- Cascaded (separate speech-to-text and text-to-speech) and native
  (speech model runs the agent) modes.
- Phone numbers and WebSocket audio. WebRTC from a browser is the only
  transport today.
