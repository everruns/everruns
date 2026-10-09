//! Voice contracts: speech-to-speech ("realtime") provider sessions and the
//! voice channel configuration shared by the Framework, serve and the server.
//!
//! Decisions (see `knowledge/framework/voice-agents.md`):
//! - A realtime provider session is a *voice front*: it listens, detects the
//!   end of the user's turn, and speaks. The Everruns agent does the thinking
//!   through its normal turn path (the `delegated` mode). The front never
//!   executes tools on its own.
//! - Events and commands are vendor-neutral so the voice loop in core never
//!   sees a wire format. A vendor driver maps its protocol onto
//!   [`RealtimeEvent`] and [`RealtimeCommand`].
//! - Provider-owned media (browser WebRTC straight to the vendor) is the first
//!   transport: Everruns only proxies the SDP offer and then drives the call
//!   over a server-side control connection, so no audio passes through it.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::runtime_provider::ProviderEndpoint;

/// How a voice channel splits listening, thinking and speaking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceMode {
    /// A speech-to-speech model listens and speaks; the Everruns agent, with
    /// its own model and tools, writes every answer. The default.
    #[default]
    Delegated,
}

/// What happens to a running agent turn when the caller starts talking over
/// the spoken answer. Speech always stops; this only decides the turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interruption {
    /// Keep the turn running; the caller's next utterance steers it.
    #[default]
    Steer,
    /// Cancel the turn; the caller's next utterance starts a new one.
    Cancel,
}

/// How the speech model decides that the caller has finished talking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDetection {
    /// Silence-based voice activity detection.
    #[default]
    ServerVad,
    /// Model-based end-of-turn detection; waits through mid-sentence pauses.
    SemanticVad,
}

/// Default speech model for OpenAI voice channels.
pub const DEFAULT_REALTIME_MODEL: &str = "gpt-realtime-2";
/// Default OpenAI voice.
pub const DEFAULT_VOICE: &str = "marin";
/// Default delay before a filler line when a turn has said nothing yet.
pub const DEFAULT_FILLER_AFTER_MS: u64 = 1_500;
/// Default filler line.
pub const DEFAULT_FILLER: &str = "One moment.";

/// Everything a voice channel decides about talking.
///
/// Stored as the platform `voice` channel's config, built in code by the
/// Framework's `VoiceChannel` and by serve's voice channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VoiceChannelConfig {
    /// Listening/thinking split. Only `delegated` today.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "delegated"))]
    pub mode: VoiceMode,
    /// Speech model id, e.g. `gpt-realtime-2`.
    #[serde(default = "default_model")]
    pub model: String,
    /// Provider voice, e.g. `marin`.
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Optional BCP 47 language hint for transcription, e.g. `en`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Spoken when the call connects. Phone lines should always set one that
    /// says the caller is talking to an AI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greeting: Option<String>,
    /// End-of-turn detection.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "server_vad"))]
    pub turn_detection: TurnDetection,
    /// What a barge-in does to the running turn.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "steer"))]
    pub interruption: Interruption,
    /// Milliseconds of silence after a turn starts before the filler line is
    /// spoken. `0` disables fillers.
    #[serde(default = "default_filler_after_ms")]
    pub filler_after_ms: u64,
    /// Filler line spoken while the agent works.
    #[serde(default = "default_filler")]
    pub filler: String,
    /// Extra speaking-style instructions for the speech model. Business rules
    /// belong in the agent, not here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking_style: Option<String>,
}

fn default_model() -> String {
    DEFAULT_REALTIME_MODEL.to_string()
}
fn default_voice() -> String {
    DEFAULT_VOICE.to_string()
}
fn default_filler_after_ms() -> u64 {
    DEFAULT_FILLER_AFTER_MS
}
fn default_filler() -> String {
    DEFAULT_FILLER.to_string()
}

impl Default for VoiceChannelConfig {
    fn default() -> Self {
        Self {
            mode: VoiceMode::default(),
            model: default_model(),
            voice: default_voice(),
            language: None,
            greeting: None,
            turn_detection: TurnDetection::default(),
            interruption: Interruption::default(),
            filler_after_ms: DEFAULT_FILLER_AFTER_MS,
            filler: default_filler(),
            speaking_style: None,
        }
    }
}

impl VoiceChannelConfig {
    /// Validate user-supplied values.
    pub fn validate(&self) -> Result<(), String> {
        let check = |name: &str, value: &str, max: usize| {
            if value.trim().is_empty() {
                Err(format!("{name} must not be empty"))
            } else if value.len() > max {
                Err(format!("{name} must be at most {max} characters"))
            } else {
                Ok(())
            }
        };
        check("model", &self.model, 128)?;
        check("voice", &self.voice, 64)?;
        check("filler", &self.filler, 200)?;
        if let Some(greeting) = &self.greeting {
            check("greeting", greeting, 500)?;
        }
        if let Some(style) = &self.speaking_style {
            check("speaking_style", style, 4_000)?;
        }
        if let Some(language) = &self.language {
            check("language", language, 16)?;
        }
        if self.filler_after_ms > 60_000 {
            return Err("filler_after_ms must be at most 60000".into());
        }
        Ok(())
    }

    /// Instructions for the speech model in delegated mode. It only speaks
    /// text the agent wrote, so these are about delivery, not content.
    pub fn speech_instructions(&self) -> String {
        let mut text = String::from(
            "You are the voice of an assistant. You never answer on your own: \
             you only speak text you are given, exactly as written, in a natural \
             conversational voice. Do not add facts, greetings or follow-up questions.",
        );
        if let Some(style) = &self.speaking_style {
            text.push_str("\n\nSpeaking style: ");
            text.push_str(style.trim());
        }
        text
    }
}

/// Session settings sent to the speech provider when a call starts.
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeSessionConfig {
    pub model: String,
    pub voice: String,
    pub instructions: String,
    pub language: Option<String>,
    pub turn_detection: TurnDetection,
    /// Stable, privacy-preserving end-user id for provider abuse monitoring
    /// (OpenAI `OpenAI-Safety-Identifier`). Never browser-supplied.
    pub safety_identifier: Option<String>,
}

impl RealtimeSessionConfig {
    pub fn from_channel(config: &VoiceChannelConfig) -> Self {
        Self {
            model: config.model.clone(),
            voice: config.voice.clone(),
            instructions: config.speech_instructions(),
            language: config.language.clone(),
            turn_detection: config.turn_detection,
            safety_identifier: None,
        }
    }

    pub fn with_safety_identifier(mut self, id: impl Into<String>) -> Self {
        self.safety_identifier = Some(id.into());
        self
    }
}

/// A WebRTC call accepted by the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeCall {
    /// SDP answer for the browser.
    pub answer_sdp: String,
    /// Provider call id used to attach the control connection. `None` when
    /// the provider did not report one; the call then has no control loop.
    pub call_id: Option<String>,
}

/// Neutral events from a speech provider's control connection.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RealtimeEvent {
    /// The caller started talking (barge-in signal).
    SpeechStarted,
    /// The caller stopped talking.
    SpeechStopped,
    /// Partial transcript of the caller's utterance.
    InputTranscriptDelta { item_id: String, delta: String },
    /// Final transcript of one caller utterance.
    InputTranscriptCompleted { item_id: String, transcript: String },
    /// Partial transcript of speech the provider is producing.
    OutputTranscriptDelta { response_id: String, delta: String },
    /// Transcript of a finished piece of speech.
    OutputTranscriptCompleted {
        response_id: String,
        transcript: String,
    },
    /// A spoken response finished. `cancelled` is true when it was cut off.
    ResponseDone {
        response_id: String,
        cancelled: bool,
    },
    /// A non-fatal provider error.
    Error { message: String },
}

/// Neutral commands to a speech provider's control connection.
// Exhaustive on purpose: every driver must implement every command.
#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeCommand {
    /// Speak `text` exactly, without adding to the provider's conversation.
    Speak { text: String },
    /// Stop the current spoken response and drop queued audio.
    StopSpeaking,
}

/// Errors from a realtime driver.
#[derive(Debug, thiserror::Error)]
pub enum RealtimeDriverError {
    #[error("realtime provider returned an error: {0}")]
    Provider(String),
    #[error("realtime request failed: {0}")]
    Transport(String),
    #[error("realtime request is invalid: {0}")]
    Invalid(String),
}

/// Control connection to one live provider call.
#[async_trait]
pub trait RealtimeConnection: Send {
    /// Next event, or `None` when the provider closed the call.
    async fn next_event(&mut self) -> Option<Result<RealtimeEvent, RealtimeDriverError>>;
    /// Send a command.
    async fn send(&mut self, command: RealtimeCommand) -> Result<(), RealtimeDriverError>;
    /// Close the connection.
    async fn close(&mut self);
}

/// Boxed control connection.
pub type BoxedRealtimeConnection = Box<dyn RealtimeConnection>;

/// Speech-to-speech provider service (`ServiceKind::Realtime`).
#[async_trait]
pub trait RealtimeDriver: Send + Sync {
    /// Accept a browser WebRTC offer; media then flows browser to provider.
    async fn accept_webrtc(
        &self,
        endpoint: &ProviderEndpoint,
        session: &RealtimeSessionConfig,
        offer_sdp: &str,
    ) -> Result<RealtimeCall, RealtimeDriverError>;

    /// Open the server-side control connection to a live call.
    async fn attach(
        &self,
        endpoint: &ProviderEndpoint,
        call_id: &str,
    ) -> Result<BoxedRealtimeConnection, RealtimeDriverError>;
}

/// Shared realtime driver handle.
pub type SharedRealtimeDriver = Arc<dyn RealtimeDriver>;

impl fmt::Debug for dyn RealtimeDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealtimeDriver")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_round_trip_from_empty_json() {
        let config: VoiceChannelConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, VoiceChannelConfig::default());
        assert_eq!(config.mode, VoiceMode::Delegated);
        assert_eq!(config.model, DEFAULT_REALTIME_MODEL);
        assert_eq!(config.interruption, Interruption::Steer);
        config.validate().unwrap();
    }

    #[test]
    fn config_rejects_bad_values() {
        let blank = VoiceChannelConfig {
            voice: " ".into(),
            ..Default::default()
        };
        assert!(blank.validate().unwrap_err().contains("voice"));
        let long = VoiceChannelConfig {
            greeting: Some("x".repeat(501)),
            ..Default::default()
        };
        assert!(long.validate().unwrap_err().contains("greeting"));
        let slow = VoiceChannelConfig {
            filler_after_ms: 60_001,
            ..Default::default()
        };
        assert!(slow.validate().is_err());
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let err = serde_json::from_str::<VoiceChannelConfig>(r#"{"mode":"telepathy"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn speech_instructions_carry_style_but_no_business_rules() {
        let config = VoiceChannelConfig {
            speaking_style: Some("Warm and brief.".into()),
            ..Default::default()
        };
        let text = config.speech_instructions();
        assert!(text.contains("exactly as written"));
        assert!(text.ends_with("Warm and brief."));
    }
}
