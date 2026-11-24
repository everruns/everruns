//! Cascaded TTS over Realtime: one isolated, audio-only response per socket.
//! The speech model is asked to read the agent's text, never given tools or
//! conversation history. This expresses exact-text intent, not a guarantee
//! that a generative speech model will reproduce every word verbatim.

use std::time::Duration;

use base64::Engine as _;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    AUDIO_SAMPLE_RATE, RealtimeCommand, RealtimeDriverError, SpeechAudio, SpeechRequest,
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::realtime::{Socket, command_events, connect_ws, realtime_ws_url};
use super::speech::Samples;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SPEECH_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_DELTA_BYTES: usize = 1_048_576;
const MARKER: &str = "cascaded_speech";

pub(super) fn uses_realtime(model: &str) -> bool {
    model == "gpt-realtime" || model.starts_with("gpt-realtime-")
}

fn session(request: &SpeechRequest) -> Value {
    json!({
        "type": "session.update",
        "session": {
            "type": "realtime",
            "model": request.model,
            "output_modalities": ["audio"],
            "tools": [],
            "audio": {
                "input": { "turn_detection": null },
                "output": {
                    "format": { "type": "audio/pcm", "rate": AUDIO_SAMPLE_RATE },
                    "voice": request.voice,
                },
            },
        },
    })
}

fn speak(request: &SpeechRequest) -> Value {
    let mut event = command_events(&RealtimeCommand::Speak {
        text: request.text.clone(),
    })
    .remove(0);
    if let Some(style) = request.instructions.as_deref().filter(|s| !s.is_empty()) {
        event["response"]["instructions"] = json!(format!(
            "Speaking style: {style}\n\n{}",
            event["response"]["instructions"]
                .as_str()
                .unwrap_or_default()
        ));
    }
    event["response"]["metadata"] = json!({ "everruns": MARKER });
    event
}

pub(super) async fn synthesize(
    endpoint: &ProviderEndpoint,
    request: &SpeechRequest,
) -> Result<SpeechAudio, RealtimeDriverError> {
    let base = endpoint
        .base_url()
        .ok_or_else(|| RealtimeDriverError::Invalid("provider has no base URL".into()))?;
    let mut url = reqwest::Url::parse(&realtime_ws_url(base))
        .map_err(|_| RealtimeDriverError::Invalid("invalid realtime base URL".into()))?;
    url.query_pairs_mut().append_pair("model", &request.model);
    let mut socket = tokio::time::timeout(
        CONNECT_TIMEOUT,
        connect_ws(endpoint, url.as_str(), request.safety_identifier.as_deref()),
    )
    .await
    .map_err(|_| RealtimeDriverError::Transport("speech connection timed out".into()))??;
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        for event in [session(request), speak(request)] {
            socket
                .send(Message::Text(event.to_string().into()))
                .await
                .map_err(|_| RealtimeDriverError::Transport("speech request send failed".into()))?;
        }
        Ok::<_, RealtimeDriverError>(())
    })
    .await
    .map_err(|_| RealtimeDriverError::Transport("speech request send timed out".into()))??;

    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut response_id = None;
        // THREAT[TM-VOICE-007]: no abandoned or stalled response keeps generating.
        let result = tokio::select! {
            result = read_pcm(&mut socket, &tx, &mut response_id) => result,
            _ = tokio::time::sleep(SPEECH_TIMEOUT) =>
                Err(RealtimeDriverError::Transport("speech response timed out".into())),
            _ = tx.closed() => {
                finish(&mut socket, response_id.as_deref(), true).await;
                return;
            }
        };
        finish(&mut socket, response_id.as_deref(), result.is_err()).await;
        if let Err(error) = result {
            let _ = tx.send(Err(error)).await;
        }
    });
    Ok(rx)
}

async fn finish(socket: &mut Socket, response_id: Option<&str>, cancel: bool) {
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        if cancel {
            let mut event = json!({ "type": "response.cancel" });
            if let Some(id) = response_id {
                event["response_id"] = json!(id);
            }
            let _ = socket.send(Message::Text(event.to_string().into())).await;
        }
        let _ = socket.close(None).await;
    })
    .await;
}

async fn read_pcm(
    socket: &mut Socket,
    tx: &mpsc::Sender<Result<Vec<u8>, RealtimeDriverError>>,
    response_id: &mut Option<String>,
) -> Result<(), RealtimeDriverError> {
    let mut samples = Samples::default();
    let mut has_audio = false;
    loop {
        let value = match socket.next().await {
            Some(Ok(Message::Text(body))) => serde_json::from_str::<Value>(&body)
                .map_err(|_| provider_error("malformed speech event"))?,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return Err(provider_error("speech stream ended before completion")),
        };
        match value["type"].as_str() {
            Some("response.created")
                if value
                    .pointer("/response/metadata/everruns")
                    .and_then(Value::as_str)
                    == Some(MARKER) =>
            {
                let id = value
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| provider_error("speech response has no id"))?;
                if response_id.as_deref().is_some_and(|active| active != id) {
                    return Err(provider_error("unexpected speech response"));
                }
                *response_id = Some(id.to_owned());
            }
            Some("response.output_audio.delta" | "response.audio.delta") => {
                let id = value["response_id"]
                    .as_str()
                    .ok_or_else(|| provider_error("speech audio has no response id"))?;
                if response_id.as_deref() != Some(id) {
                    continue;
                }
                let delta = value["delta"]
                    .as_str()
                    .filter(|delta| delta.len() <= MAX_DELTA_BYTES)
                    .ok_or_else(|| provider_error("invalid speech audio delta"))?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(delta)
                    .map_err(|_| provider_error("invalid speech audio encoding"))?;
                let frame = samples.cut(&bytes);
                if !frame.is_empty() {
                    has_audio = true;
                    if tx.send(Ok(frame)).await.is_err() {
                        return Err(provider_error("speech playback cancelled"));
                    }
                }
            }
            Some("response.done")
                if response_id.is_some()
                    && value.pointer("/response/id").and_then(Value::as_str)
                        == response_id.as_deref() =>
            {
                if value.pointer("/response/status").and_then(Value::as_str) != Some("completed") {
                    return Err(provider_error("speech response did not complete"));
                }
                if !has_audio || samples.carry.is_some() {
                    return Err(provider_error("speech response has no complete PCM audio"));
                }
                return Ok(());
            }
            // THREAT[TM-VOICE-005]: errors can echo text or credentials; never forward them.
            Some("error") => return Err(provider_error("speech provider rejected the request")),
            _ => {}
        }
    }
}

fn provider_error(message: &'static str) -> RealtimeDriverError {
    RealtimeDriverError::Provider(message.into())
}

#[cfg(test)]
#[path = "realtime_speech_tests.rs"]
mod tests;
