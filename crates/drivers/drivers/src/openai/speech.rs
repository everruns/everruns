//! OpenAI speech-to-text and text-to-speech for cascaded voice channels.
//!
//! [`transcribe`] opens a Realtime transcription session
//! (`/realtime?intent=transcription`): caller audio goes up as
//! `input_audio_buffer.append`, and OpenAI's voice activity detection and
//! transcription events come back in the same shape a speech-to-speech call
//! reports them, so they map through [`map_server_event`]. [`synthesize`]
//! streams `/audio/speech` as raw PCM.
//!
//! Decisions:
//! - 24 kHz 16-bit mono PCM both ways, the format both endpoints take
//!   natively, so no audio is resampled here.
//! - Synthesized audio is re-cut to whole samples, so a host can play each
//!   frame on its own.
//! - Keys, audio and raw provider payloads are never logged.
//!
//! [`transcribe`]: SpeechDriver::transcribe
//! [`synthesize`]: SpeechDriver::synthesize
//! [`map_server_event`]: super::realtime::map_server_event

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    AUDIO_SAMPLE_RATE, RealtimeDriverError, RealtimeEvent, SpeechAudio, SpeechDriver,
    SpeechRequest, TranscriptionConfig, TranscriptionStream, TurnDetection,
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::realtime::{auth_headers, connect_ws, endpoint_url, map_server_event, realtime_ws_url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// One spoken chunk is a sentence or two; this bounds a stalled stream.
const SPEECH_TIMEOUT: Duration = Duration::from_secs(60);
const CHANNEL_BUFFER: usize = 64;

/// OpenAI speech-to-text and text-to-speech driver.
#[derive(Debug, Clone, Default)]
pub struct OpenAISpeechDriver {
    http: reqwest::Client,
}

impl OpenAISpeechDriver {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(SPEECH_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }
}

/// The `session.update` that configures a transcription session.
pub fn transcription_session(config: &TranscriptionConfig) -> Value {
    let mut transcription = json!({ "model": config.model });
    if let Some(language) = &config.language {
        transcription["language"] = json!(language);
    }
    let turn_detection = match config.turn_detection {
        TurnDetection::ServerVad => json!({ "type": "server_vad" }),
        TurnDetection::SemanticVad => json!({ "type": "semantic_vad" }),
    };
    json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": AUDIO_SAMPLE_RATE },
                    "transcription": transcription,
                    "turn_detection": turn_detection,
                }
            }
        }
    })
}

/// The `/audio/speech` request body.
pub fn speech_body(request: &SpeechRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "voice": request.voice,
        "input": request.text,
        "response_format": "pcm",
    });
    if let Some(instructions) = request.instructions.as_deref().filter(|s| !s.is_empty()) {
        body["instructions"] = json!(instructions);
    }
    body
}

/// Cuts a byte stream into whole 16-bit samples.
#[derive(Default)]
struct Samples {
    carry: Option<u8>,
}

impl Samples {
    fn cut(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.extend(self.carry.take());
        out.extend_from_slice(bytes);
        if out.len() % 2 == 1 {
            self.carry = out.pop();
        }
        out
    }
}

#[async_trait]
impl SpeechDriver for OpenAISpeechDriver {
    async fn transcribe(
        &self,
        endpoint: &ProviderEndpoint,
        config: &TranscriptionConfig,
    ) -> Result<TranscriptionStream, RealtimeDriverError> {
        let base = endpoint
            .base_url()
            .ok_or_else(|| RealtimeDriverError::Invalid("provider has no base URL".into()))?;
        let url = format!("{}?intent=transcription", realtime_ws_url(base));
        let mut socket = connect_ws(endpoint, &url, config.safety_identifier.as_deref()).await?;
        socket
            .send(WsMessage::Text(
                transcription_session(config).to_string().into(),
            ))
            .await
            .map_err(|e| RealtimeDriverError::Transport(e.to_string()))?;

        let (audio, mut caller) = mpsc::channel::<Vec<u8>>(CHANNEL_BUFFER);
        let (events_tx, events) = mpsc::channel(CHANNEL_BUFFER);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    frame = caller.recv() => {
                        let Some(frame) = frame else {
                            // The caller hung up.
                            let _ = socket.close(None).await;
                            return;
                        };
                        let append = json!({
                            "type": "input_audio_buffer.append",
                            "audio": base64::engine::general_purpose::STANDARD.encode(&frame),
                        });
                        if let Err(error) = socket.send(WsMessage::Text(append.to_string().into())).await {
                            let _ = events_tx.send(Err(RealtimeDriverError::Transport(error.to_string()))).await;
                            return;
                        }
                    }
                    message = socket.next() => {
                        let message = match message {
                            None | Some(Ok(WsMessage::Close(_))) => return,
                            Some(Err(error)) => {
                                let _ = events_tx.send(Err(RealtimeDriverError::Transport(error.to_string()))).await;
                                return;
                            }
                            Some(Ok(message)) => message,
                        };
                        let WsMessage::Text(body) = message else { continue };
                        let Ok(value) = serde_json::from_str::<Value>(&body) else { continue };
                        if let Some(event) = transcription_event(&value)
                            && events_tx.send(Ok(event)).await.is_err()
                        {
                            let _ = socket.close(None).await;
                            return;
                        }
                    }
                }
            }
        });
        Ok(TranscriptionStream { audio, events })
    }

    async fn synthesize(
        &self,
        endpoint: &ProviderEndpoint,
        request: &SpeechRequest,
    ) -> Result<SpeechAudio, RealtimeDriverError> {
        if request.text.trim().is_empty() {
            return Err(RealtimeDriverError::Invalid("nothing to say".into()));
        }
        let url = endpoint_url(endpoint, "audio/speech")?;
        let mut http = self.http.post(&url).json(&speech_body(request));
        for (name, value) in auth_headers(endpoint, "POST", &url).await? {
            http = http.header(name, value);
        }
        if let Some(id) = &request.safety_identifier {
            http = http.header("OpenAI-Safety-Identifier", id);
        }
        let response = http
            .send()
            .await
            .map_err(|e| RealtimeDriverError::Transport(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            // The body can echo the request; report the status only.
            return Err(RealtimeDriverError::Provider(format!(
                "speech request rejected with status {status}"
            )));
        }
        let (tx, rx) = mpsc::channel(CHANNEL_BUFFER);
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut samples = Samples::default();
            while let Some(chunk) = stream.next().await {
                let item = chunk
                    .map(|bytes| samples.cut(&bytes))
                    .map_err(|e| RealtimeDriverError::Transport(e.without_url().to_string()));
                let failed = item.is_err();
                if tx.send(item).await.is_err() || failed {
                    return;
                }
            }
        });
        Ok(rx)
    }
}

/// The events a transcription session reports: voice activity, transcripts
/// and errors.
fn transcription_event(value: &Value) -> Option<RealtimeEvent> {
    map_server_event(value).filter(|event| {
        matches!(
            event,
            RealtimeEvent::SpeechStarted
                | RealtimeEvent::SpeechStopped
                | RealtimeEvent::InputTranscriptDelta { .. }
                | RealtimeEvent::InputTranscriptCompleted { .. }
                | RealtimeEvent::Error { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::voice::VoiceChannelConfig;

    #[test]
    fn transcription_session_takes_pcm_and_the_channel_settings() {
        let mut config = TranscriptionConfig::from_channel(&VoiceChannelConfig {
            language: Some("uk".into()),
            turn_detection: TurnDetection::SemanticVad,
            ..Default::default()
        });
        config.model = "gpt-4o-mini-transcribe".into();
        let update = transcription_session(&config);
        assert_eq!(update["type"], "session.update");
        let input = &update["session"]["audio"]["input"];
        assert_eq!(update["session"]["type"], "transcription");
        assert_eq!(input["format"], json!({"type": "audio/pcm", "rate": 24000}));
        assert_eq!(input["transcription"]["model"], "gpt-4o-mini-transcribe");
        assert_eq!(input["transcription"]["language"], "uk");
        assert_eq!(input["turn_detection"]["type"], "semantic_vad");
    }

    #[test]
    fn speech_body_asks_for_raw_pcm() {
        let body = speech_body(&SpeechRequest {
            model: "gpt-4o-mini-tts".into(),
            voice: "marin".into(),
            text: "Hello.".into(),
            instructions: Some("Warm.".into()),
            safety_identifier: None,
        });
        assert_eq!(body["response_format"], "pcm");
        assert_eq!(body["input"], "Hello.");
        assert_eq!(body["instructions"], "Warm.");
        let plain = speech_body(&SpeechRequest {
            model: "m".into(),
            voice: "v".into(),
            text: "t".into(),
            instructions: Some(String::new()),
            safety_identifier: None,
        });
        assert!(plain.get("instructions").is_none());
    }

    #[test]
    fn audio_is_cut_to_whole_samples() {
        let mut samples = Samples::default();
        assert_eq!(samples.cut(&[1, 2, 3]), vec![1, 2]);
        assert_eq!(samples.cut(&[4]), vec![3, 4]);
        assert_eq!(samples.cut(&[5, 6]), vec![5, 6]);
    }

    #[test]
    fn only_caller_side_events_pass() {
        assert_eq!(
            transcription_event(&json!({"type": "input_audio_buffer.speech_started"})),
            Some(RealtimeEvent::SpeechStarted)
        );
        assert_eq!(
            transcription_event(&json!({
                "type": "conversation.item.input_audio_transcription.completed",
                "item_id": "i1",
                "transcript": "Hi"
            })),
            Some(RealtimeEvent::InputTranscriptCompleted {
                item_id: "i1".into(),
                transcript: "Hi".into()
            })
        );
        assert_eq!(
            transcription_event(&json!({"type": "response.done", "response": {"id": "r"}})),
            None
        );
    }
}
