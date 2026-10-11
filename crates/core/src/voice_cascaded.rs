//! Cascaded voice: a speech-to-text and a text-to-speech service presented to
//! the voice loop as one [`RealtimeConnection`].
//!
//! Decisions:
//! - The loop stays mode-agnostic. Transcription events pass through as they
//!   are; `Speak` synthesizes the text and streams its audio to the host;
//!   `StopSpeaking` aborts synthesis and tells the host to drop audio it has
//!   not played yet ([`AudioOut::Clear`]).
//! - Audio is paced at real time with a short lead, so the host never holds
//!   more than a fraction of a second of unplayed speech and a barge-in cuts
//!   off almost at once. A response is reported done once its last audio is
//!   handed over, which lets the next sentence synthesize while the tail
//!   plays.
//! - A text-to-speech service reports no transcript of what played, so what
//!   the caller heard is estimated from playback time. The loop only uses it
//!   to split a cut-off answer into heard and unspoken text.
//! - Events of a stopped response are dropped by response id, so a late event
//!   from aborted synthesis can never be taken for the next response.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    AUDIO_BYTES_PER_SECOND, BoxedRealtimeConnection, RealtimeCommand, RealtimeConnection,
    RealtimeDriverError, RealtimeEvent, SharedSpeechDriver, SpeechRequest, TranscriptionConfig,
    VoiceChannelConfig,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// How far ahead of playback audio is handed to the host.
const LEAD: Duration = Duration::from_millis(300);
/// Speaking rate used to estimate how much of a response the caller heard
/// before its audio length is known.
const CHARS_PER_SECOND: f64 = 15.0;
/// Audio frames buffered toward the host before synthesis waits.
const AUDIO_OUT_BUFFER: usize = 64;
/// Caller audio frames buffered toward the transcription service.
const AUDIO_IN_BUFFER: usize = 64;

/// Audio for the host to play to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioOut {
    /// 16-bit PCM, mono, 24 kHz, to play after what came before.
    Audio(Vec<u8>),
    /// Drop audio not played yet: the caller talked over it.
    Clear,
}

/// A cascaded call's pieces: the connection the voice loop drives, and the
/// two audio channels the host's transport connects to the caller.
pub struct CascadedCall {
    /// Hand to [`VoiceLoop::run`](super::VoiceLoop::run).
    pub connection: BoxedRealtimeConnection,
    /// Caller audio in, 16-bit PCM mono at 24 kHz. Dropping it ends the
    /// transcription, and with it the call.
    pub audio_in: mpsc::Sender<Vec<u8>>,
    /// Speech out, in playback order.
    pub audio_out: mpsc::Receiver<AudioOut>,
}

/// Open a cascaded call on `driver`: start transcribing the caller and get
/// ready to speak with the channel's voice and speech model.
///
/// `safety_identifier` is a stable, privacy-preserving end-user id for the
/// provider's abuse monitoring.
pub async fn connect(
    driver: SharedSpeechDriver,
    endpoint: ProviderEndpoint,
    config: &VoiceChannelConfig,
    safety_identifier: Option<String>,
) -> Result<CascadedCall, RealtimeDriverError> {
    let mut transcription = TranscriptionConfig::from_channel(config);
    transcription.safety_identifier = safety_identifier.clone();
    let stream = driver.transcribe(&endpoint, &transcription).await?;
    let (audio_out_tx, audio_out) = mpsc::channel(AUDIO_OUT_BUFFER);
    let template = SpeechRequest {
        model: config.speech_model().to_string(),
        voice: config.voice.clone(),
        text: String::new(),
        instructions: config
            .speaking_style
            .as_ref()
            .map(|style| style.trim().to_string()),
        safety_identifier,
    };
    // Relay caller audio through a channel of our own so the host's sender
    // stays usable however the driver buffers.
    let (audio_in, mut caller) = mpsc::channel::<Vec<u8>>(AUDIO_IN_BUFFER);
    let to_driver = stream.audio;
    tokio::spawn(async move {
        while let Some(frame) = caller.recv().await {
            if to_driver.send(frame).await.is_err() {
                return;
            }
        }
    });
    let connection =
        CascadedConnection::new(stream.events, driver, endpoint, template, audio_out_tx);
    Ok(CascadedCall {
        connection: Box::new(connection),
        audio_in,
        audio_out,
    })
}

/// The voice loop's view of a cascaded call. Built by [`connect`].
pub struct CascadedConnection {
    transcription: mpsc::Receiver<Result<RealtimeEvent, RealtimeDriverError>>,
    driver: SharedSpeechDriver,
    endpoint: ProviderEndpoint,
    template: SpeechRequest,
    audio_out: mpsc::Sender<AudioOut>,
    responses_tx: mpsc::UnboundedSender<RealtimeEvent>,
    responses: mpsc::UnboundedReceiver<RealtimeEvent>,
    /// The response being spoken: its id and synthesis task.
    current: Option<(String, JoinHandle<()>)>,
    /// When handed-over audio finishes playing.
    playing_until: Arc<Mutex<Option<Instant>>>,
    next_id: u64,
}

impl CascadedConnection {
    fn new(
        transcription: mpsc::Receiver<Result<RealtimeEvent, RealtimeDriverError>>,
        driver: SharedSpeechDriver,
        endpoint: ProviderEndpoint,
        template: SpeechRequest,
        audio_out: mpsc::Sender<AudioOut>,
    ) -> Self {
        let (responses_tx, responses) = mpsc::unbounded_channel();
        Self {
            transcription,
            driver,
            endpoint,
            template,
            audio_out,
            responses_tx,
            responses,
            current: None,
            playing_until: Arc::default(),
            next_id: 0,
        }
    }

    fn is_current(&self, event: &RealtimeEvent) -> bool {
        let id = match event {
            RealtimeEvent::OutputTranscriptDelta { response_id, .. }
            | RealtimeEvent::OutputTranscriptCompleted { response_id, .. }
            | RealtimeEvent::ResponseDone { response_id, .. } => response_id,
            _ => return true,
        };
        self.current
            .as_ref()
            .is_some_and(|(current, _)| current == id)
    }

    /// Stop synthesis and drop unplayed audio.
    async fn stop(&mut self) {
        if let Some((_, task)) = self.current.take() {
            task.abort();
        }
        let playing = lock(&self.playing_until)
            .take()
            .is_some_and(|until| until > Instant::now());
        if playing {
            let _ = self.audio_out.send(AudioOut::Clear).await;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[async_trait]
impl RealtimeConnection for CascadedConnection {
    async fn next_event(&mut self) -> Option<Result<RealtimeEvent, RealtimeDriverError>> {
        loop {
            tokio::select! {
                Some(event) = self.responses.recv() => {
                    if !self.is_current(&event) {
                        continue;
                    }
                    if matches!(event, RealtimeEvent::ResponseDone { .. }) {
                        self.current = None;
                    }
                    return Some(Ok(event));
                }
                event = self.transcription.recv() => {
                    // Talking over the tail of a finished response: the loop
                    // no longer counts it as speaking, so cut the tail here.
                    if matches!(event, Some(Ok(RealtimeEvent::SpeechStarted)))
                        && self.current.is_none()
                    {
                        self.stop().await;
                    }
                    return event;
                }
            }
        }
    }

    async fn send(&mut self, command: RealtimeCommand) -> Result<(), RealtimeDriverError> {
        match command {
            RealtimeCommand::Speak { text } => {
                // The next sentence queues after the previous one's tail;
                // only a barge-in drops unplayed audio.
                if let Some((_, task)) = self.current.take() {
                    task.abort();
                }
                self.next_id += 1;
                let id = format!("speech_{}", self.next_id);
                let request = SpeechRequest {
                    text,
                    ..self.template.clone()
                };
                let task = tokio::spawn(speak(Speaking {
                    driver: self.driver.clone(),
                    endpoint: self.endpoint.clone(),
                    request,
                    id: id.clone(),
                    audio_out: self.audio_out.clone(),
                    events: self.responses_tx.clone(),
                    playing_until: self.playing_until.clone(),
                }));
                self.current = Some((id, task));
            }
            RealtimeCommand::StopSpeaking => self.stop().await,
        }
        Ok(())
    }

    async fn close(&mut self) {
        if let Some((_, task)) = self.current.take() {
            task.abort();
        }
        self.transcription.close();
    }
}

impl Drop for CascadedConnection {
    fn drop(&mut self) {
        if let Some((_, task)) = self.current.take() {
            task.abort();
        }
    }
}

struct Speaking {
    driver: SharedSpeechDriver,
    endpoint: ProviderEndpoint,
    request: SpeechRequest,
    id: String,
    audio_out: mpsc::Sender<AudioOut>,
    events: mpsc::UnboundedSender<RealtimeEvent>,
    playing_until: Arc<Mutex<Option<Instant>>>,
}

fn audio_duration(bytes: u64) -> Duration {
    Duration::from_secs_f64(bytes as f64 / f64::from(AUDIO_BYTES_PER_SECOND))
}

/// Synthesize one response and hand its audio over at playback pace.
async fn speak(speaking: Speaking) {
    let Speaking {
        driver,
        endpoint,
        request,
        id,
        audio_out,
        events,
        playing_until,
    } = speaking;
    let text = request.text.clone();
    let mut heard = HeardEstimate::new(&text);
    let emit_heard = |heard: &mut HeardEstimate, upto: usize| {
        if let Some(delta) = heard.advance(upto) {
            let _ = events.send(RealtimeEvent::OutputTranscriptDelta {
                response_id: id.clone(),
                delta,
            });
        }
    };
    match driver.synthesize(&endpoint, &request).await {
        Err(error) => {
            let _ = events.send(RealtimeEvent::Error {
                message: error.to_string(),
            });
        }
        Ok(mut audio) => {
            let mut started: Option<Instant> = None;
            let mut sent: u64 = 0;
            while let Some(chunk) = audio.recv().await {
                let bytes = match chunk {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = events.send(RealtimeEvent::Error {
                            message: error.to_string(),
                        });
                        break;
                    }
                };
                if bytes.is_empty() {
                    continue;
                }
                let start = *started.get_or_insert_with(Instant::now);
                sent += bytes.len() as u64;
                if audio_out.send(AudioOut::Audio(bytes)).await.is_err() {
                    // The caller's transport is gone; the call is ending.
                    return;
                }
                let ends = start + audio_duration(sent);
                *lock(&playing_until) = Some(ends);
                tokio::time::sleep_until(ends.checked_sub(LEAD).unwrap_or(start)).await;
                let upto = heard.chars_after(start.elapsed().as_secs_f64());
                emit_heard(&mut heard, upto);
            }
        }
    }
    emit_heard(&mut heard, usize::MAX);
    let _ = events.send(RealtimeEvent::OutputTranscriptCompleted {
        response_id: id.clone(),
        transcript: text,
    });
    let _ = events.send(RealtimeEvent::ResponseDone {
        response_id: id,
        cancelled: false,
    });
}

/// What the caller has heard of one response, cut at word boundaries.
struct HeardEstimate {
    chars: Vec<char>,
    reported: usize,
}

impl HeardEstimate {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            reported: 0,
        }
    }

    /// Characters heard after `seconds` of playback.
    fn chars_after(&self, seconds: f64) -> usize {
        (seconds * CHARS_PER_SECOND) as usize
    }

    /// Report up to `upto` characters, backed up to the last whole word.
    /// `None` when there is nothing new.
    fn advance(&mut self, upto: usize) -> Option<String> {
        let total = self.chars.len();
        let mut end = upto.min(total);
        if end < total {
            while end > self.reported && !self.chars[end].is_whitespace() {
                end -= 1;
            }
        }
        if end <= self.reported {
            return None;
        }
        let delta: String = self.chars[self.reported..end].iter().collect();
        self.reported = end;
        Some(delta)
    }
}

#[cfg(test)]
#[path = "voice_cascaded_tests.rs"]
mod tests;
