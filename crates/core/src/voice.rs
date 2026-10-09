//! The voice loop: one implementation of "talk to an agent" shared by the
//! Framework facade, serve and the server.
//!
//! Decisions:
//! - Voice is a channel. The loop never owns an agent: it turns the caller's
//!   final transcripts into ordinary session messages through
//!   [`VoiceSessionPort`], and speaks the agent's output as it streams in.
//! - Delegated mode: the speech model only listens and speaks. Every word it
//!   says is text the agent wrote (or the channel's greeting and filler line).
//! - Speech goes out sentence by sentence, one spoken response at a time, so
//!   the first sentence is heard while the agent is still writing the rest.
//! - Barge-in always stops speech. [`Interruption`] only decides what happens
//!   to the running turn. Output of an interrupted answer stays muted until the
//!   agent starts a new message after the caller's next utterance, so a steered
//!   turn never finishes reading the stale answer.
//! - The loop is transport neutral: hosts feed it a
//!   [`RealtimeConnection`](everruns_contracts::voice::RealtimeConnection) and
//!   a channel of [`AgentOutput`] mapped from their own event stream.

use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::voice::{
    BoxedRealtimeConnection, Interruption, RealtimeCommand, RealtimeDriverError, RealtimeEvent,
    VoiceChannelConfig,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Longest chunk spoken without a sentence boundary.
const MAX_CHUNK_CHARS: usize = 280;

/// The session side of a voice call, implemented by each host.
#[async_trait]
pub trait VoiceSessionPort: Send + Sync {
    /// Deliver one final caller utterance as a user message. Starts a turn
    /// when the session is idle and steers the running turn otherwise.
    async fn send_utterance(&self, text: &str) -> Result<(), VoiceLoopError>;

    /// Cancel the running turn, if any. Used when the channel's
    /// interruption policy is [`Interruption::Cancel`].
    async fn cancel_turn(&self) -> Result<(), VoiceLoopError>;

    /// Record what happened on the call (transcripts, interruptions, latency).
    /// Hosts map these onto their own event log; failures are theirs to log.
    async fn record(&self, event: VoiceLoopEvent);
}

/// Agent output the host forwards from the session's event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentOutput {
    /// The agent started a new assistant message.
    MessageStarted,
    /// More text of the current assistant message.
    TextDelta(String),
    /// A complete statement to speak as is, such as tool narration.
    Commentary(String),
    /// The turn finished, failed or was cancelled.
    TurnEnded,
}

/// What the loop reports to [`VoiceSessionPort::record`].
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceLoopEvent {
    InputTranscriptDelta {
        item_id: String,
        delta: String,
    },
    InputTranscriptCompleted {
        item_id: String,
        transcript: String,
    },
    OutputTranscriptDelta {
        response_id: String,
        delta: String,
    },
    OutputTranscriptCompleted {
        response_id: String,
        transcript: String,
    },
    /// The caller talked over the answer. `heard` is what was spoken before
    /// speech stopped; `unspoken` is the answer text that was dropped.
    OutputInterrupted {
        heard: String,
        unspoken: String,
        policy: Interruption,
    },
    /// First spoken words of an answer (filler excluded), measured from the
    /// end of the caller's utterance.
    AnswerStarted {
        latency_ms: u64,
    },
    /// A provider error that did not end the call.
    ProviderError {
        message: String,
    },
}

/// Errors that end a voice loop.
#[derive(Debug, thiserror::Error)]
pub enum VoiceLoopError {
    #[error(transparent)]
    Realtime(#[from] RealtimeDriverError),
    #[error("voice session error: {0}")]
    Session(String),
}

/// Counters returned when a call ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceLoopSummary {
    pub utterances: u32,
    pub spoken_chunks: u32,
    pub interruptions: u32,
}

/// Runs one call. Construct per call and drive with [`VoiceLoop::run`].
pub struct VoiceLoop<P> {
    config: VoiceChannelConfig,
    port: P,
    state: LoopState,
}

#[derive(Default)]
struct LoopState {
    /// Agent text not yet cut into a speakable chunk.
    pending: String,
    /// Chunks waiting for the current spoken response to finish.
    queue: VecDeque<Chunk>,
    /// A spoken response is in flight.
    speaking: bool,
    /// What the caller heard of the in-flight response.
    heard_current: String,
    /// What the caller heard of the current answer before this response.
    heard_answer: String,
    /// Text of the in-flight response, to report the unspoken tail.
    current_text: String,
    /// Output of an interrupted answer is dropped until a new message.
    muted: bool,
    /// An utterance was sent since the last barge-in, so the next message
    /// is the answer to it.
    unmute_on_message: bool,
    /// The agent is working on an utterance and has not finished the turn.
    turn_open: bool,
    /// When the last utterance ended, for latency and filler.
    utterance_at: Option<Instant>,
    /// Whether anything was spoken for the current utterance.
    answered: bool,
    filler_deadline: Option<Instant>,
    summary: VoiceLoopSummary,
}

#[derive(Debug, Clone)]
struct Chunk {
    text: String,
    /// Part of the agent's answer (not greeting or filler).
    answer: bool,
}

impl<P: VoiceSessionPort> VoiceLoop<P> {
    pub fn new(config: VoiceChannelConfig, port: P) -> Self {
        Self {
            config,
            port,
            state: LoopState::default(),
        }
    }

    /// Drive the call until the provider closes it, the agent output channel
    /// and the call both end, or `shutdown` fires. Closes the connection.
    pub async fn run(
        mut self,
        mut connection: BoxedRealtimeConnection,
        mut agent: mpsc::Receiver<AgentOutput>,
        shutdown: CancellationToken,
    ) -> Result<VoiceLoopSummary, VoiceLoopError> {
        let result = self.drive(&mut connection, &mut agent, &shutdown).await;
        connection.close().await;
        result.map(|()| self.state.summary)
    }

    async fn drive(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
        agent: &mut mpsc::Receiver<AgentOutput>,
        shutdown: &CancellationToken,
    ) -> Result<(), VoiceLoopError> {
        if let Some(greeting) = self.config.greeting.clone() {
            self.enqueue(
                connection,
                Chunk {
                    text: greeting,
                    answer: false,
                },
            )
            .await?;
        }
        let mut agent_open = true;
        loop {
            let filler_at = self.state.filler_deadline;
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(()),
                event = connection.next_event() => match event {
                    None => return Ok(()),
                    Some(Err(err)) => return Err(err.into()),
                    Some(Ok(event)) => self.on_provider_event(connection, event).await?,
                },
                output = agent.recv(), if agent_open => match output {
                    Some(output) => self.on_agent_output(connection, output).await?,
                    None => agent_open = false,
                },
                () = sleep_until_opt(filler_at), if filler_at.is_some() => {
                    self.state.filler_deadline = None;
                    if self.state.turn_open && !self.state.answered && !self.state.speaking {
                        let filler = self.config.filler.clone();
                        self.enqueue(connection, Chunk { text: filler, answer: false }).await?;
                    }
                }
            }
        }
    }

    async fn on_provider_event(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
        event: RealtimeEvent,
    ) -> Result<(), VoiceLoopError> {
        match event {
            RealtimeEvent::SpeechStarted => self.on_barge_in(connection).await?,
            RealtimeEvent::InputTranscriptDelta { item_id, delta } => {
                self.port
                    .record(VoiceLoopEvent::InputTranscriptDelta { item_id, delta })
                    .await;
            }
            RealtimeEvent::InputTranscriptCompleted {
                item_id,
                transcript,
            } => {
                let text = transcript.trim().to_string();
                self.port
                    .record(VoiceLoopEvent::InputTranscriptCompleted {
                        item_id,
                        transcript: transcript.clone(),
                    })
                    .await;
                if !text.is_empty() {
                    self.on_utterance(&text).await?;
                }
            }
            RealtimeEvent::OutputTranscriptDelta { response_id, delta } => {
                self.state.heard_current.push_str(&delta);
                self.port
                    .record(VoiceLoopEvent::OutputTranscriptDelta { response_id, delta })
                    .await;
            }
            RealtimeEvent::OutputTranscriptCompleted {
                response_id,
                transcript,
            } => {
                self.port
                    .record(VoiceLoopEvent::OutputTranscriptCompleted {
                        response_id,
                        transcript,
                    })
                    .await;
            }
            RealtimeEvent::ResponseDone { .. } => {
                if self.state.speaking {
                    self.state.speaking = false;
                    let heard = std::mem::take(&mut self.state.heard_current);
                    append_spoken(&mut self.state.heard_answer, &heard);
                    self.state.current_text.clear();
                    self.speak_next(connection).await?;
                }
            }
            RealtimeEvent::Error { message } => {
                self.port
                    .record(VoiceLoopEvent::ProviderError { message })
                    .await;
            }
            // SpeechStopped and future provider events need no action.
            _ => {}
        }
        Ok(())
    }

    async fn on_utterance(&mut self, text: &str) -> Result<(), VoiceLoopError> {
        self.port.send_utterance(text).await?;
        let state = &mut self.state;
        state.summary.utterances += 1;
        state.turn_open = true;
        state.answered = false;
        state.heard_answer.clear();
        state.utterance_at = Some(Instant::now());
        state.filler_deadline = (self.config.filler_after_ms > 0)
            .then(|| Instant::now() + Duration::from_millis(self.config.filler_after_ms));
        if state.muted {
            state.unmute_on_message = true;
        }
        Ok(())
    }

    async fn on_barge_in(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
    ) -> Result<(), VoiceLoopError> {
        let talking = self.state.speaking || !self.state.queue.is_empty();
        if !talking {
            return Ok(());
        }
        if self.state.speaking {
            connection.send(RealtimeCommand::StopSpeaking).await?;
        }
        let state = &mut self.state;
        state.speaking = false;
        let mut heard = std::mem::take(&mut state.heard_answer);
        append_spoken(&mut heard, &std::mem::take(&mut state.heard_current));
        let mut unspoken = unspoken_tail(&std::mem::take(&mut state.current_text), &heard);
        for chunk in state.queue.drain(..).filter(|chunk| chunk.answer) {
            append_spoken(&mut unspoken, &chunk.text);
        }
        append_spoken(&mut unspoken, std::mem::take(&mut state.pending).trim());
        state.muted = true;
        state.unmute_on_message = false;
        state.filler_deadline = None;
        state.summary.interruptions += 1;
        let policy = self.config.interruption;
        self.port
            .record(VoiceLoopEvent::OutputInterrupted {
                heard,
                unspoken,
                policy,
            })
            .await;
        if policy == Interruption::Cancel && self.state.turn_open {
            self.state.turn_open = false;
            self.port.cancel_turn().await?;
        }
        Ok(())
    }

    async fn on_agent_output(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
        output: AgentOutput,
    ) -> Result<(), VoiceLoopError> {
        match output {
            AgentOutput::MessageStarted => {
                if self.state.muted && self.state.unmute_on_message {
                    self.state.muted = false;
                    self.state.unmute_on_message = false;
                }
                // A new message starts a new sentence.
                self.flush(connection, true).await?;
            }
            AgentOutput::TextDelta(delta) => {
                if !self.state.muted {
                    self.state.pending.push_str(&delta);
                    self.flush(connection, false).await?;
                }
            }
            AgentOutput::Commentary(text) => {
                if !self.state.muted {
                    self.flush(connection, true).await?;
                    if let Some(text) = speakable(&text) {
                        self.enqueue(connection, Chunk { text, answer: true })
                            .await?;
                    }
                }
            }
            AgentOutput::TurnEnded => {
                if !self.state.muted {
                    self.flush(connection, true).await?;
                }
                self.state.pending.clear();
                self.state.turn_open = false;
                self.state.filler_deadline = None;
            }
        }
        Ok(())
    }

    /// Cut complete sentences (or everything, when `all`) out of `pending`.
    async fn flush(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
        all: bool,
    ) -> Result<(), VoiceLoopError> {
        for text in take_chunks(&mut self.state.pending, all) {
            self.enqueue(connection, Chunk { text, answer: true })
                .await?;
        }
        Ok(())
    }

    async fn enqueue(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
        chunk: Chunk,
    ) -> Result<(), VoiceLoopError> {
        self.state.queue.push_back(chunk);
        if !self.state.speaking {
            self.speak_next(connection).await?;
        }
        Ok(())
    }

    async fn speak_next(
        &mut self,
        connection: &mut BoxedRealtimeConnection,
    ) -> Result<(), VoiceLoopError> {
        let Some(chunk) = self.state.queue.pop_front() else {
            return Ok(());
        };
        connection
            .send(RealtimeCommand::Speak {
                text: chunk.text.clone(),
            })
            .await?;
        let state = &mut self.state;
        state.speaking = true;
        state.current_text = chunk.text;
        state.summary.spoken_chunks += 1;
        if chunk.answer && !state.answered {
            state.answered = true;
            state.filler_deadline = None;
            if let Some(at) = state.utterance_at.take() {
                let latency_ms = u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX);
                self.port
                    .record(VoiceLoopEvent::AnswerStarted { latency_ms })
                    .await;
            }
        }
        Ok(())
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn append_spoken(into: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !into.is_empty() {
        into.push(' ');
    }
    into.push_str(text);
}

/// The part of `text` the caller did not hear, given the transcript of what
/// they did. Transcripts are close to, not exactly, the spoken text, so this
/// cuts by length rather than by matching.
fn unspoken_tail(text: &str, heard_of_it: &str) -> String {
    let heard_chars = heard_of_it.chars().count();
    let total = text.chars().count();
    if heard_chars >= total {
        return String::new();
    }
    // Back up to a word start so a half-heard word counts as unspoken.
    let mut tail: String = text.chars().skip(heard_chars).collect();
    let mid_word = |i: usize| text.chars().nth(i).is_some_and(|c| !c.is_whitespace());
    if heard_chars > 0 && mid_word(heard_chars - 1) && mid_word(heard_chars) {
        let head: String = text.chars().take(heard_chars).collect();
        let word_start = head.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        tail = format!("{}{}", &head[word_start..], tail);
    }
    tail.trim().to_string()
}

/// Cut speakable chunks from `buf`, leaving an incomplete sentence behind
/// unless `all` is set.
pub fn take_chunks(buf: &mut String, all: bool) -> Vec<String> {
    let mut chunks = Vec::new();
    loop {
        let cut = sentence_end(buf).or_else(|| {
            (buf.len() > MAX_CHUNK_CHARS).then(|| {
                let limit = floor_char_boundary(buf, MAX_CHUNK_CHARS);
                buf[..limit]
                    .rfind(char::is_whitespace)
                    .map_or(limit, |i| i + 1)
            })
        });
        let Some(cut) = cut else { break };
        let head: String = buf.drain(..cut).collect();
        if let Some(text) = speakable(&head) {
            chunks.push(text);
        }
    }
    if all {
        let rest = std::mem::take(buf);
        if let Some(text) = speakable(&rest) {
            chunks.push(text);
        }
    }
    chunks
}

/// Byte index just past the first sentence boundary: `.`, `!`, `?` or `:`
/// followed by whitespace, or a newline.
fn sentence_end(buf: &str) -> Option<usize> {
    let mut chars = buf.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\n' {
            return Some(i + 1);
        }
        if matches!(c, '.' | '!' | '?' | ':' | ';')
            && let Some(&(j, next)) = chars.peek()
            && next.is_whitespace()
        {
            return Some(j + next.len_utf8());
        }
    }
    None
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Strip Markdown that reads badly aloud; `None` when nothing is left to say.
pub fn speakable(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim();
        let line = line.trim_start_matches('#').trim_start();
        let line = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        let cleaned: String = line
            .chars()
            .filter(|c| !matches!(c, '*' | '`' | '|'))
            .collect();
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(cleaned);
    }
    let has_words = out.chars().any(char::is_alphanumeric);
    has_words.then_some(out)
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
