//! Voice channels: let people talk to a [`Session`] and hear its answers.
//!
//! A [`VoiceChannel`](crate::voice::VoiceChannel) holds everything about talking (speech model, voice,
//! greeting, turn-taking, interruption) and the speech provider it uses. The
//! agent stays unchanged: the same session can take typed messages and calls,
//! and both land in one transcript.
//!
//! Calls run in **delegated** mode: a speech-to-speech model only listens and
//! speaks, and the agent, with its own model and tools, writes every answer.
//! Answers are spoken sentence by sentence while they stream, a filler line
//! covers a slow turn, and talking over the answer stops speech at once.
//!
//! ```no_run
//! # #[cfg(feature = "openai")]
//! # async fn demo(engine: everruns::Engine, agent: everruns::Agent, offer_sdp: String)
//! # -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::providers::openai::OpenAI;
//! use everruns::voice::{Interruption, VoiceChannel};
//!
//! let voice = VoiceChannel::delegated(OpenAI::from_env()?.realtime())
//!     .voice("marin")
//!     .greeting("Hi, you are talking to an AI assistant. How can I help?")
//!     .interruption(Interruption::Steer);
//!
//! let session = engine.create(agent);
//! // A browser's WebRTC offer in, the SDP answer out. Audio flows between the
//! // browser and the speech provider; the session hears every utterance.
//! let call = voice.accept_webrtc(&session, &offer_sdp).await?;
//! let answer_sdp = call.answer_sdp().to_string();
//! # let _ = answer_sdp;
//! call.end().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Stability: experimental — outside the [`stability`](crate::stability)
//! promises.
//!
//! Decisions:
//! - The loop is `everruns_core::voice::VoiceLoop`, the one the everruns
//!   server and serve run, so a call behaves the same everywhere.
//! - A caller utterance is an ordinary [`Session::send`]: it starts a turn when
//!   the session is idle and steers the running turn otherwise. It carries
//!   `metadata.source = "voice"` so a transcript view can mark it.
//! - Only WebRTC with server-side SDP exchange today; audio never passes
//!   through this process.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_core::voice::{AgentOutput, VoiceLoop, VoiceLoopError, VoiceSessionPort};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{InputMessage, Session, SessionEventKind, TurnHandle};

pub use everruns_contracts::voice::{
    Interruption, RealtimeDriver, RealtimeDriverError, SharedRealtimeDriver, TurnDetection,
    VoiceChannelConfig, VoiceMode,
};
pub use everruns_core::voice::{
    VoiceLoopEvent as VoiceCallEvent, VoiceLoopSummary as VoiceCallSummary,
};
/// The caller's side of a call placed on [`Realtime::simulated`]: say
/// utterances, hang up, and read back what was spoken. For tests and demos.
pub use everruns_llmsim::realtime::SimulatedCall;

/// How many call events a slow [`VoiceCall::events`] reader may fall behind
/// before it sees a lag.
const EVENT_BUFFER: usize = 256;

/// A speech-to-speech provider: the driver and where it connects.
///
/// Build one from a provider, such as
/// [`OpenAI::realtime`](crate::providers::openai::OpenAI::realtime) with the
/// `openai` feature, or use [`Realtime::simulated`] to run calls offline.
#[derive(Clone)]
pub struct Realtime {
    driver: SharedRealtimeDriver,
    endpoint: ProviderEndpoint,
}

impl Realtime {
    /// A provider from any [`RealtimeDriver`] and its endpoint.
    pub fn new(driver: impl RealtimeDriver + 'static, endpoint: ProviderEndpoint) -> Self {
        Self {
            driver: Arc::new(driver),
            endpoint,
        }
    }

    /// The offline simulator. It never touches the network: a test plays the
    /// caller through [`SimulatedCall`], found by [`VoiceCall::call_id`].
    pub fn simulated() -> Self {
        Self::new(
            everruns_llmsim::realtime::LlmSimRealtimeDriver,
            ProviderEndpoint::default(),
        )
    }
}

impl std::fmt::Debug for Realtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Realtime").finish_non_exhaustive()
    }
}

/// Why a call could not be placed or ended cleanly.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VoiceError {
    /// The channel settings are invalid.
    #[error("invalid voice channel: {0}")]
    Invalid(String),
    /// The speech provider refused or failed the call.
    #[error(transparent)]
    Provider(#[from] RealtimeDriverError),
    /// The provider accepted the call but returned no call id, so it cannot
    /// be driven.
    #[error("the speech provider returned no call id")]
    NoCallId,
    /// The call ended with an error.
    #[error("voice call failed: {0}")]
    Call(String),
}

/// A voice channel: how calls to a session sound and behave.
#[derive(Clone, Debug)]
pub struct VoiceChannel {
    realtime: Realtime,
    config: VoiceChannelConfig,
}

impl VoiceChannel {
    /// A delegated channel on `realtime`: the speech model listens and
    /// speaks, the agent writes every answer. Defaults match the platform's
    /// voice channel.
    pub fn delegated(realtime: Realtime) -> Self {
        Self {
            realtime,
            config: VoiceChannelConfig::default(),
        }
    }

    /// A channel from a complete config, as stored on a platform voice
    /// channel.
    pub fn with_config(realtime: Realtime, config: VoiceChannelConfig) -> Self {
        Self { realtime, config }
    }

    /// Speech model id, such as `gpt-realtime-2`.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Provider voice, such as `marin`.
    pub fn voice(mut self, voice: impl Into<String>) -> Self {
        self.config.voice = voice.into();
        self
    }

    /// Language hint for transcribing the caller, such as `en`.
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.config.language = Some(language.into());
        self
    }

    /// Spoken when the call connects. Say that the caller is talking to an AI.
    pub fn greeting(mut self, greeting: impl Into<String>) -> Self {
        self.config.greeting = Some(greeting.into());
        self
    }

    /// How the end of the caller's turn is detected.
    pub fn turn_detection(mut self, turn_detection: TurnDetection) -> Self {
        self.config.turn_detection = turn_detection;
        self
    }

    /// What talking over the answer does to the running turn.
    pub fn interruption(mut self, interruption: Interruption) -> Self {
        self.config.interruption = interruption;
        self
    }

    /// The line spoken while the agent works.
    pub fn filler(mut self, filler: impl Into<String>) -> Self {
        self.config.filler = filler.into();
        self
    }

    /// Silence before the filler is spoken. Zero turns fillers off.
    pub fn filler_after(mut self, after: Duration) -> Self {
        self.config.filler_after_ms = u64::try_from(after.as_millis()).unwrap_or(u64::MAX);
        self
    }

    /// Delivery instructions for the speech model, such as pace or tone.
    /// Business rules belong in the agent.
    pub fn speaking_style(mut self, style: impl Into<String>) -> Self {
        self.config.speaking_style = Some(style.into());
        self
    }

    /// The channel's settings.
    pub fn config(&self) -> &VoiceChannelConfig {
        &self.config
    }

    /// Answer a browser's WebRTC offer and start a call on `session`.
    ///
    /// Returns once the provider accepted the call; the call then runs in
    /// the background until the caller hangs up, [`VoiceCall::end`] is
    /// called, or the handle is dropped.
    ///
    /// # Errors
    ///
    /// [`VoiceError::Invalid`] for bad settings or an empty offer, and
    /// [`VoiceError::Provider`] when the provider refuses the call.
    pub async fn accept_webrtc(
        &self,
        session: &Session,
        offer_sdp: &str,
    ) -> Result<VoiceCall, VoiceError> {
        self.accept_webrtc_as(session, offer_sdp, None).await
    }

    /// [`accept_webrtc`](Self::accept_webrtc) with a stable, privacy-preserving
    /// end-user id the provider uses for abuse monitoring (OpenAI's
    /// `OpenAI-Safety-Identifier`). Never pass raw personal data.
    pub async fn accept_webrtc_as(
        &self,
        session: &Session,
        offer_sdp: &str,
        safety_identifier: Option<&str>,
    ) -> Result<VoiceCall, VoiceError> {
        if offer_sdp.trim().is_empty() {
            return Err(VoiceError::Invalid("empty SDP offer".into()));
        }
        self.config.validate().map_err(VoiceError::Invalid)?;
        let mut speech =
            everruns_contracts::voice::RealtimeSessionConfig::from_channel(&self.config);
        if let Some(id) = safety_identifier {
            speech = speech.with_safety_identifier(id);
        }
        let Realtime { driver, endpoint } = &self.realtime;
        let accepted = driver.accept_webrtc(endpoint, &speech, offer_sdp).await?;
        let call_id = accepted.call_id.ok_or(VoiceError::NoCallId)?;
        let connection = driver.attach(endpoint, &call_id).await?;

        // Subscribe before the loop starts so no answer is missed.
        let mut session_events = session.events();
        let (agent_tx, agent_rx) = mpsc::channel(256);
        let (events, first_reader) = broadcast::channel(EVENT_BUFFER);
        let shutdown = CancellationToken::new();

        let pump_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut mapper = OutputMapper::default();
            loop {
                tokio::select! {
                    () = pump_shutdown.cancelled() => return,
                    event = session_events.recv() => match event {
                        Ok(Some(event)) => {
                            for output in mapper.map(&event.kind, event.canonical_json()) {
                                if agent_tx.send(output).await.is_err() {
                                    return;
                                }
                            }
                        }
                        // A lagged reader skips; the loop recovers at the next turn.
                        Err(_) => continue,
                        Ok(None) => return,
                    }
                }
            }
        });

        let port = FrameworkPort {
            session: session.clone(),
            turn: Mutex::new(None),
            events: events.clone(),
        };
        let loop_shutdown = shutdown.clone();
        let config = self.config.clone();
        let task = tokio::spawn(async move {
            let result = VoiceLoop::new(config, port)
                .run(connection, agent_rx, loop_shutdown.clone())
                .await;
            // Stops the output pump too.
            loop_shutdown.cancel();
            result
        });

        Ok(VoiceCall {
            answer_sdp: accepted.answer_sdp,
            call_id,
            events,
            first_reader: Mutex::new(Some(first_reader)),
            shutdown,
            task: Some(task),
        })
    }
}

/// A running call. Dropping it ends the call.
pub struct VoiceCall {
    answer_sdp: String,
    call_id: String,
    events: broadcast::Sender<VoiceCallEvent>,
    first_reader: Mutex<Option<broadcast::Receiver<VoiceCallEvent>>>,
    shutdown: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<VoiceCallSummary, VoiceLoopError>>>,
}

impl VoiceCall {
    /// The SDP answer to hand back to the browser.
    pub fn answer_sdp(&self) -> &str {
        &self.answer_sdp
    }

    /// The provider's call id.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Transcripts, interruptions, latency and provider errors of this call.
    ///
    /// The first reader receives every event since the call started; later
    /// readers receive events from the moment they subscribe.
    pub fn events(&self) -> broadcast::Receiver<VoiceCallEvent> {
        self.first_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| self.events.subscribe())
    }

    /// Whether the call has ended.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(|task| task.is_finished())
    }

    /// Wait until the caller hangs up.
    ///
    /// # Errors
    ///
    /// [`VoiceError::Call`] when the call ended with an error.
    pub async fn wait(mut self) -> Result<VoiceCallSummary, VoiceError> {
        self.join().await
    }

    /// Hang up and wait for the call to finish.
    ///
    /// # Errors
    ///
    /// [`VoiceError::Call`] when the call had already failed.
    pub async fn end(mut self) -> Result<VoiceCallSummary, VoiceError> {
        self.shutdown.cancel();
        self.join().await
    }

    async fn join(&mut self) -> Result<VoiceCallSummary, VoiceError> {
        let Some(task) = self.task.take() else {
            return Ok(VoiceCallSummary::default());
        };
        match task.await {
            Ok(Ok(summary)) => Ok(summary),
            Ok(Err(error)) => Err(VoiceError::Call(error.to_string())),
            Err(error) => Err(VoiceError::Call(error.to_string())),
        }
    }
}

impl Drop for VoiceCall {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl std::fmt::Debug for VoiceCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceCall")
            .field("call_id", &self.call_id)
            .finish_non_exhaustive()
    }
}

/// The session side of a call.
struct FrameworkPort {
    session: Session,
    /// The turn the latest utterance started or steered.
    turn: Mutex<Option<TurnHandle>>,
    events: broadcast::Sender<VoiceCallEvent>,
}

#[async_trait]
impl VoiceSessionPort for FrameworkPort {
    async fn send_utterance(&self, text: &str) -> Result<(), VoiceLoopError> {
        let mut message = InputMessage::user(text);
        message.metadata = Some([("source".to_string(), json!("voice"))].into());
        let sent = self
            .session
            .send(message)
            .await
            .map_err(|error| VoiceLoopError::Session(error.to_string()))?;
        *self
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sent.turn());
        Ok(())
    }

    async fn cancel_turn(&self) -> Result<(), VoiceLoopError> {
        let turn = self
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(turn) = turn {
            // A turn that already finished has nothing to cancel.
            let _ = turn.cancel().await;
        }
        Ok(())
    }

    async fn record(&self, event: VoiceCallEvent) {
        let _ = self.events.send(event);
    }
}

/// Session events to voice loop input.
#[derive(Default)]
struct OutputMapper {
    /// The current output message streamed text deltas.
    streamed: bool,
}

impl OutputMapper {
    fn map(&mut self, kind: &SessionEventKind, canonical: &Value) -> Vec<AgentOutput> {
        match kind {
            SessionEventKind::OutputStarted { .. } => {
                self.streamed = false;
                vec![AgentOutput::MessageStarted]
            }
            SessionEventKind::TextDelta { delta } => {
                self.streamed = true;
                vec![AgentOutput::TextDelta(delta.clone())]
            }
            SessionEventKind::OutputCompleted { .. } => {
                // Drivers that do not stream still produce a spoken answer.
                if std::mem::take(&mut self.streamed) {
                    return Vec::new();
                }
                let text = completed_text(canonical);
                if text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![AgentOutput::MessageStarted, AgentOutput::TextDelta(text)]
                }
            }
            SessionEventKind::TurnCompleted
            | SessionEventKind::TurnFailed { .. }
            | SessionEventKind::TurnCancelled => vec![AgentOutput::TurnEnded],
            _ => Vec::new(),
        }
    }
}

/// The text parts of a completed output message's canonical envelope.
fn completed_text(canonical: &Value) -> String {
    canonical
        .pointer("/data/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
