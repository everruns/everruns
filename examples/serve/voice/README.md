# voice (serve, experimental)

A [serve](../../../crates/serve) app people can call from the browser. The
`voice` feature gives every top-level agent a voice channel at
`/v1/channels/{agent}/voice`: the agent stays the same one typed messages
reach, and `[voice]` in `serve.toml` sets how calls sound.

```sh
OPENAI_API_KEY=… cargo run -p serve-example-voice   # dev server on :3000
cargo run -p serve-example-voice -- eval            # 1 passed
```

Open <http://127.0.0.1:3000/v1/channels/concierge/voice> and select
**Start call**. Ask "Do you have a room on Friday?". The greeting plays when
the call connects, the agent checks `rooms`, and its answer is spoken
sentence by sentence while it streams. Talk over it to interrupt.

Calls run in delegated mode: OpenAI's `gpt-realtime-2` only listens and
speaks, and the agent, on its own model and tools, writes every answer. The
key stays on the server; the browser exchanges its WebRTC offer through
`POST /v1/channels/concierge/voice/calls` and audio flows straight to OpenAI.

Without `OPENAI_API_KEY`, `dev` places calls on the offline speech simulator:
the routes answer, but no audio plays. `start` refuses calls without a key.

## Call it from your own page

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
// Later: POST /v1/channels/concierge/voice/calls/{call_id}/end
```

The call's session is an ordinary session: follow its transcript at
`/v1/sessions/{session_id}/sse`, type into it at `/v1/sessions/{session_id}/messages`,
or pass `session_id` to the next call to keep talking where the chat left off.
Each utterance is a user message with `metadata.source = "voice"`.

| File | Is |
|---|---|
| `serve.toml` | `[voice]`: voice, greeting, speaking style |
| `agent/instructions.md` | the always-on prompt, written for speech |
| `src/agent.rs` | `#[agent] fn concierge()` |
| `src/tools/rooms.rs` | `#[tool] async fn rooms(night)` |
| `evals/rooms.rs` | `#[eval] async fn checks_rooms_before_answering(t)` |
