---
title: Voice
description: Let people talk to an Everruns agent and hear its answers through a voice channel, with the same model, tools and instructions the agent uses for text.
sidebar:
  label: Voice
appliesTo: [platform, cloud]
---

A **voice channel** lets people talk to an Agent and hear its answers. Voice is
a [channel](/features/channels/) like Slack or AG-UI, so the same Agent can be
reachable by text and by voice at once, with one set of instructions, tools and
memory. A call and a text chat on the same session share one transcript, so a
conversation can move between typing and talking.

> **Status:** Voice is an Adoption feature. An organization administrator turns
> on **Voice** in **Settings → Features**. Speech uses an OpenAI provider.

## How a call works

A voice channel runs in **delegated** mode. A speech-to-speech model
(`gpt-realtime-2` by default) only listens and speaks; the Agent, with its own
model and tools, writes every answer.

1. The browser connects to the speech provider over WebRTC. Everruns places the
   call server-side, so the provider key never reaches the browser.
2. When the caller finishes talking, their words become a user message on the
   session, marked with `metadata.source = "voice"`.
3. The Agent runs a normal turn. Its answer is spoken sentence by sentence as it
   streams, so speech starts before the answer is complete.
4. If the turn says nothing for a moment, a short filler line ("One moment.") is
   spoken so the line does not feel dead.
5. When the caller talks over the answer, speech stops at once. The transcript
   records what the caller actually heard.

Tools, approvals, budgets and audit apply to a voice turn exactly as to a text
turn. Raw audio is never stored; transcripts are kept like chat messages.

## Create a voice channel

1. Open the Agent and select the **Integrations** tab.
2. Select **Add channel** and choose **Voice**.
3. Pick a voice and, optionally, a greeting, a language hint and a speaking
   style, then select **Save channel**.
4. Select **Talk to this channel** to test it from the browser.

Over the API, create it like any channel:

```bash
curl -X POST "$EVERRUNS_API/v1/agents/$AGENT_ID/channels" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "channel_type": "voice",
    "channel_config": {
      "voice": "marin",
      "greeting": "Hi, you are talking to an AI assistant. How can I help?"
    }
  }'
```

## Settings

| Field | Default | What it does |
|---|---|---|
| `mode` | `delegated` | How listening, thinking and speaking are split. Only `delegated` today. |
| `model` | `gpt-realtime-2` | Speech model that listens and speaks. |
| `voice` | `marin` | Provider voice. |
| `language` | none | Language hint for transcribing the caller, such as `en`. |
| `greeting` | none | Spoken when the call connects. Say that the caller is talking to an AI. |
| `turn_detection` | `server_vad` | `server_vad` ends a turn on a pause; `semantic_vad` waits for a finished thought. |
| `interruption` | `steer` | What talking over the answer does to the running turn: `steer` keeps it running and the next utterance steers it, `cancel` stops it. |
| `filler` | `One moment.` | Line spoken while the Agent works. |
| `filler_after_ms` | `1500` | Silence before the filler is spoken. `0` turns fillers off. |
| `speaking_style` | none | Delivery instructions for the speech model, such as pace or tone. Business rules belong in the Agent. |

The speech provider is the organization's default for realtime speech. Pass
`provider_id` when placing a call to use another configured OpenAI provider.

## Place a call from your own app

Your app creates a WebRTC offer with the microphone track, sends it to
Everruns, and applies the SDP answer it gets back.

| Method | Route | Use |
|---|---|---|
| `POST` | `/v1/agents/{agent_id}/channels/{channel_id}/voice/calls` | Call a voice channel. Starts a new session unless `session_id` is given. |
| `POST` | `/v1/sessions/{session_id}/voice/calls` | Talk on an existing session. Uses the Agent's voice channel, or `channel_id` if given. |
| `POST` | `/v1/sessions/{session_id}/voice/{voice_connection_id}/end` | Hang up. |

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
const response = await fetch(
  `${api}/v1/agents/${agentId}/channels/${channelId}/voice/calls`,
  {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ sdp: offer.sdp }),
  },
);
const { session, voice } = await response.json();
await pc.setRemoteDescription({ type: "answer", sdp: voice.answer_sdp });
```

The response carries the session, so your app can show the transcript by
[streaming its events](/how-to/stream-events/). A call lasts at most one hour.

## Events

A call emits `voice.session.started`, `voice.session.ended` and
`voice.session.failed`, transcript events for the caller
(`voice.input_transcript.*`) and for what was spoken
(`voice.output_transcript.*`), and `voice.output.interrupted` when the caller
talks over an answer. See the [event reference](/event-reference/).

## See also

- [Channels](/features/channels/): the channel model, lifecycle and publishing.
- [Stream events](/how-to/stream-events/): follow a session's transcript live.
- [Build a voice agent](/framework/voice/): the same voice channel in your own
  Rust host or a serve app.
