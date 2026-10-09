//! OpenAI Realtime voice driver (`ServiceKind::Realtime`).
//!
//! Browser media goes straight to OpenAI over WebRTC: [`accept_webrtc`] posts
//! the browser's SDP offer with the session settings to `/realtime/calls` and
//! returns OpenAI's answer. [`attach`] then opens the server-side control
//! WebSocket for that call (`/realtime?call_id=…`), which carries transcripts
//! and the "speak this" commands. No audio passes through Everruns.
//!
//! Decisions:
//! - Automatic responses are off (`create_response: false`). The speech model
//!   never answers on its own; the Everruns agent writes every answer and the
//!   voice loop sends it back as an out-of-band, audio-only response
//!   (`conversation: "none"`), so the provider's conversation never diverges
//!   from the durable transcript.
//! - No tools are advertised to the speech model. Tools run in the agent turn
//!   under Everruns permissions.
//! - SDP bodies, keys and raw provider payloads are never logged.
//!
//! [`accept_webrtc`]: RealtimeDriver::accept_webrtc
//! [`attach`]: RealtimeDriver::attach

use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    BoxedRealtimeConnection, RealtimeCall, RealtimeCommand, RealtimeConnection, RealtimeDriver,
    RealtimeDriverError, RealtimeEvent, RealtimeSessionConfig, TurnDetection,
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{
    Message as WsMessage, client::IntoClientRequest, http::HeaderValue,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIPTION_MODEL: &str = "gpt-4o-transcribe";

/// OpenAI Realtime speech-to-speech driver.
#[derive(Debug, Clone, Default)]
pub struct OpenAIRealtimeDriver {
    http: reqwest::Client,
}

impl OpenAIRealtimeDriver {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }
}

/// The `session` object sent with a new call.
pub fn session_payload(config: &RealtimeSessionConfig) -> Value {
    let mut transcription = json!({ "model": TRANSCRIPTION_MODEL });
    if let Some(language) = &config.language {
        transcription["language"] = json!(language);
    }
    let turn_detection = match config.turn_detection {
        TurnDetection::ServerVad => json!({
            "type": "server_vad",
            "create_response": false,
            "interrupt_response": true,
        }),
        TurnDetection::SemanticVad => json!({
            "type": "semantic_vad",
            "create_response": false,
            "interrupt_response": true,
        }),
    };
    json!({
        "type": "realtime",
        "model": config.model,
        "instructions": config.instructions,
        "audio": {
            "input": {
                "transcription": transcription,
                "turn_detection": turn_detection,
            },
            "output": { "voice": config.voice },
        },
        "tools": [],
    })
}

fn endpoint_url(endpoint: &ProviderEndpoint, path: &str) -> Result<String, RealtimeDriverError> {
    endpoint
        .url(path)
        .ok_or_else(|| RealtimeDriverError::Invalid("provider has no base URL".into()))
}

async fn auth_headers(
    endpoint: &ProviderEndpoint,
    method: &str,
    url: &str,
) -> Result<Vec<(String, String)>, RealtimeDriverError> {
    endpoint
        .resolve(method, url, &[])
        .await
        .map(|request| request.headers)
        .map_err(|error| RealtimeDriverError::Invalid(error.to_string()))
}

/// The call id is the last path segment of the `Location` header
/// (`/v1/realtime/calls/{call_id}`).
pub fn call_id_from_location(location: &str) -> Option<String> {
    let path = location.split(['?', '#']).next()?;
    let id = path.trim_end_matches('/').rsplit('/').next()?;
    (!id.is_empty() && id != "calls").then(|| id.to_string())
}

/// `wss://…/realtime?call_id=…` for an `https://…/v1` base URL.
pub fn control_url(base: &str, call_id: &str) -> String {
    let base = base
        .trim_end_matches('/')
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    format!("{base}/realtime?call_id={call_id}")
}

#[async_trait]
impl RealtimeDriver for OpenAIRealtimeDriver {
    async fn accept_webrtc(
        &self,
        endpoint: &ProviderEndpoint,
        session: &RealtimeSessionConfig,
        offer_sdp: &str,
    ) -> Result<RealtimeCall, RealtimeDriverError> {
        if offer_sdp.trim().is_empty() {
            return Err(RealtimeDriverError::Invalid("missing SDP offer".into()));
        }
        let url = endpoint_url(endpoint, "realtime/calls")?;
        let form = reqwest::multipart::Form::new()
            .part(
                "sdp",
                reqwest::multipart::Part::text(offer_sdp.to_string())
                    .mime_str("application/sdp")
                    .map_err(|e| RealtimeDriverError::Invalid(e.to_string()))?,
            )
            .text("session", session_payload(session).to_string());
        let mut request = self.http.post(&url).multipart(form);
        for (name, value) in auth_headers(endpoint, "POST", &url).await? {
            request = request.header(name, value);
        }
        if let Some(id) = &session.safety_identifier {
            request = request.header("OpenAI-Safety-Identifier", id);
        }
        let response = request
            .send()
            .await
            .map_err(|e| RealtimeDriverError::Transport(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            // The body can echo request details; report the status only.
            return Err(RealtimeDriverError::Provider(format!(
                "realtime call rejected with status {status}"
            )));
        }
        let call_id = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(call_id_from_location);
        let answer_sdp = response
            .text()
            .await
            .map_err(|e| RealtimeDriverError::Transport(e.without_url().to_string()))?;
        Ok(RealtimeCall {
            answer_sdp,
            call_id,
        })
    }

    async fn attach(
        &self,
        endpoint: &ProviderEndpoint,
        call_id: &str,
    ) -> Result<BoxedRealtimeConnection, RealtimeDriverError> {
        if call_id.is_empty()
            || !call_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(RealtimeDriverError::Invalid("invalid call id".into()));
        }
        let base = endpoint
            .base_url()
            .ok_or_else(|| RealtimeDriverError::Invalid("provider has no base URL".into()))?;
        let url = control_url(base, call_id);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| RealtimeDriverError::Invalid(e.to_string()))?;
        for (name, value) in auth_headers(endpoint, "GET", &url).await? {
            let name =
                tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|e| RealtimeDriverError::Invalid(e.to_string()))?;
            let value = HeaderValue::from_str(&value)
                .map_err(|e| RealtimeDriverError::Invalid(e.to_string()))?;
            request.headers_mut().insert(name, value);
        }
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| RealtimeDriverError::Transport(e.to_string()))?;
        Ok(Box::new(OpenAIRealtimeConnection { socket }))
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct OpenAIRealtimeConnection {
    socket: Socket,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Map one OpenAI Realtime server event onto the neutral event set.
pub fn map_server_event(value: &Value) -> Option<RealtimeEvent> {
    let kind = value.get("type")?.as_str()?;
    let item_id = || text(value, "item_id").unwrap_or_default();
    let response_id = || {
        text(value, "response_id")
            .or_else(|| {
                value
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default()
    };
    Some(match kind {
        "input_audio_buffer.speech_started" => RealtimeEvent::SpeechStarted,
        "input_audio_buffer.speech_stopped" => RealtimeEvent::SpeechStopped,
        "conversation.item.input_audio_transcription.delta" => {
            RealtimeEvent::InputTranscriptDelta {
                item_id: item_id(),
                delta: text(value, "delta").unwrap_or_default(),
            }
        }
        "conversation.item.input_audio_transcription.completed" => {
            RealtimeEvent::InputTranscriptCompleted {
                item_id: item_id(),
                transcript: text(value, "transcript").unwrap_or_default(),
            }
        }
        "response.output_audio_transcript.delta" | "response.audio_transcript.delta" => {
            RealtimeEvent::OutputTranscriptDelta {
                response_id: response_id(),
                delta: text(value, "delta").unwrap_or_default(),
            }
        }
        "response.output_audio_transcript.done" | "response.audio_transcript.done" => {
            RealtimeEvent::OutputTranscriptCompleted {
                response_id: response_id(),
                transcript: text(value, "transcript").unwrap_or_default(),
            }
        }
        "response.done" => {
            let status = value.pointer("/response/status").and_then(Value::as_str);
            RealtimeEvent::ResponseDone {
                response_id: response_id(),
                cancelled: matches!(status, Some("cancelled" | "incomplete")),
            }
        }
        "error" => RealtimeEvent::Error {
            message: value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("realtime provider error")
                .to_string(),
        },
        _ => return None,
    })
}

/// The OpenAI client events for one neutral command.
pub fn command_events(command: &RealtimeCommand) -> Vec<Value> {
    match command {
        RealtimeCommand::Speak { text } => vec![json!({
            "type": "response.create",
            "response": {
                "conversation": "none",
                "output_modalities": ["audio"],
                "input": [],
                "instructions": format!(
                    "Say exactly the following text, and nothing else:\n\n{}",
                    text.trim()
                ),
            }
        })],
        // WebRTC keeps unplayed audio in the provider's output buffer; clear
        // it as well as cancelling generation so speech stops at once.
        RealtimeCommand::StopSpeaking => vec![
            json!({ "type": "response.cancel" }),
            json!({ "type": "output_audio_buffer.clear" }),
        ],
    }
}

#[async_trait]
impl RealtimeConnection for OpenAIRealtimeConnection {
    async fn next_event(&mut self) -> Option<Result<RealtimeEvent, RealtimeDriverError>> {
        loop {
            let message = match self.socket.next().await? {
                Ok(message) => message,
                Err(error) => return Some(Err(RealtimeDriverError::Transport(error.to_string()))),
            };
            let WsMessage::Text(body) = message else {
                if matches!(message, WsMessage::Close(_)) {
                    return None;
                }
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            if let Some(event) = map_server_event(&value) {
                return Some(Ok(event));
            }
        }
    }

    async fn send(&mut self, command: RealtimeCommand) -> Result<(), RealtimeDriverError> {
        for event in command_events(&command) {
            self.socket
                .send(WsMessage::Text(event.to_string().into()))
                .await
                .map_err(|e| RealtimeDriverError::Transport(e.to_string()))?;
        }
        Ok(())
    }

    async fn close(&mut self) {
        let _ = self.socket.close(None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::voice::VoiceChannelConfig;

    #[test]
    fn session_payload_never_answers_alone_or_advertises_tools() {
        let config = RealtimeSessionConfig::from_channel(&VoiceChannelConfig {
            language: Some("en".into()),
            ..Default::default()
        });
        let payload = session_payload(&config);
        assert_eq!(payload["model"], "gpt-realtime-2");
        assert_eq!(payload["tools"], json!([]));
        let detection = &payload["audio"]["input"]["turn_detection"];
        assert_eq!(detection["create_response"], false);
        assert_eq!(detection["interrupt_response"], true);
        assert_eq!(payload["audio"]["input"]["transcription"]["language"], "en");
        assert_eq!(payload["audio"]["output"]["voice"], "marin");
    }

    #[test]
    fn semantic_vad_maps_to_provider_type() {
        let config = RealtimeSessionConfig::from_channel(&VoiceChannelConfig {
            turn_detection: TurnDetection::SemanticVad,
            ..Default::default()
        });
        assert_eq!(
            session_payload(&config)["audio"]["input"]["turn_detection"]["type"],
            "semantic_vad"
        );
    }

    #[test]
    fn call_id_parsing() {
        assert_eq!(
            call_id_from_location("/v1/realtime/calls/rtc_abc123").as_deref(),
            Some("rtc_abc123")
        );
        assert_eq!(
            call_id_from_location("https://api.openai.com/v1/realtime/calls/rtc_x?y=1").as_deref(),
            Some("rtc_x")
        );
        assert_eq!(call_id_from_location("/v1/realtime/calls/"), None);
    }

    #[test]
    fn control_url_switches_scheme() {
        assert_eq!(
            control_url("https://api.openai.com/v1/", "rtc_1"),
            "wss://api.openai.com/v1/realtime?call_id=rtc_1"
        );
        assert_eq!(
            control_url("http://127.0.0.1:9/v1", "c"),
            "ws://127.0.0.1:9/v1/realtime?call_id=c"
        );
    }

    #[test]
    fn maps_server_events() {
        let event = map_server_event(&json!({
            "type": "conversation.item.input_audio_transcription.completed",
            "item_id": "item_1",
            "transcript": "book Tuesday"
        }));
        assert_eq!(
            event,
            Some(RealtimeEvent::InputTranscriptCompleted {
                item_id: "item_1".into(),
                transcript: "book Tuesday".into()
            })
        );
        assert_eq!(
            map_server_event(&json!({"type": "input_audio_buffer.speech_started"})),
            Some(RealtimeEvent::SpeechStarted)
        );
        assert_eq!(
            map_server_event(
                &json!({"type": "response.done", "response": {"id": "r1", "status": "cancelled"}})
            ),
            Some(RealtimeEvent::ResponseDone {
                response_id: "r1".into(),
                cancelled: true
            })
        );
        assert_eq!(
            map_server_event(
                &json!({"type": "response.output_audio_transcript.delta", "response_id": "r2", "delta": "Hi"})
            ),
            Some(RealtimeEvent::OutputTranscriptDelta {
                response_id: "r2".into(),
                delta: "Hi".into()
            })
        );
        assert_eq!(map_server_event(&json!({"type": "session.updated"})), None);
    }

    #[test]
    fn speak_is_out_of_band_and_audio_only() {
        let events = command_events(&RealtimeCommand::Speak {
            text: " Hello. ".into(),
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["response"]["conversation"], "none");
        assert_eq!(events[0]["response"]["output_modalities"], json!(["audio"]));
        assert!(
            events[0]["response"]["instructions"]
                .as_str()
                .unwrap()
                .ends_with("Hello.")
        );
        let stop = command_events(&RealtimeCommand::StopSpeaking);
        assert_eq!(stop[0]["type"], "response.cancel");
        assert_eq!(stop[1]["type"], "output_audio_buffer.clear");
    }
}
