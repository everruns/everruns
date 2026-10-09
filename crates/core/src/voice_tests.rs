use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::voice::{RealtimeConnection, RealtimeEvent};

use super::*;

/// Scripted provider connection: the test pushes events, and reads back the
/// commands the loop sent.
struct FakeConnection {
    events: mpsc::UnboundedReceiver<RealtimeEvent>,
    commands: mpsc::UnboundedSender<RealtimeCommand>,
}

#[async_trait]
impl RealtimeConnection for FakeConnection {
    async fn next_event(&mut self) -> Option<Result<RealtimeEvent, RealtimeDriverError>> {
        self.events.recv().await.map(Ok)
    }
    async fn send(&mut self, command: RealtimeCommand) -> Result<(), RealtimeDriverError> {
        let _ = self.commands.send(command);
        Ok(())
    }
    async fn close(&mut self) {}
}

#[derive(Debug, Clone, PartialEq)]
enum PortCall {
    Send(String),
    Cancel,
    Event(VoiceLoopEvent),
}

#[derive(Clone, Default)]
struct FakePort(Arc<Mutex<Vec<PortCall>>>);

impl FakePort {
    fn calls(&self) -> Vec<PortCall> {
        self.0.lock().unwrap().clone()
    }
    fn sends(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                PortCall::Send(text) => Some(text),
                _ => None,
            })
            .collect()
    }
    fn interruptions(&self) -> Vec<VoiceLoopEvent> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                PortCall::Event(event @ VoiceLoopEvent::OutputInterrupted { .. }) => Some(event),
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl VoiceSessionPort for FakePort {
    async fn send_utterance(&self, text: &str) -> Result<(), VoiceLoopError> {
        self.0.lock().unwrap().push(PortCall::Send(text.into()));
        Ok(())
    }
    async fn cancel_turn(&self) -> Result<(), VoiceLoopError> {
        self.0.lock().unwrap().push(PortCall::Cancel);
        Ok(())
    }
    async fn record(&self, event: VoiceLoopEvent) {
        self.0.lock().unwrap().push(PortCall::Event(event));
    }
}

struct CallRig {
    events: mpsc::UnboundedSender<RealtimeEvent>,
    commands: mpsc::UnboundedReceiver<RealtimeCommand>,
    agent: mpsc::Sender<AgentOutput>,
    port: FakePort,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<Result<VoiceLoopSummary, VoiceLoopError>>,
}

impl CallRig {
    fn start(config: VoiceChannelConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (agent_tx, agent_rx) = mpsc::channel(64);
        let port = FakePort::default();
        let shutdown = CancellationToken::new();
        let connection: BoxedRealtimeConnection = Box::new(FakeConnection {
            events: event_rx,
            commands: command_tx,
        });
        let task = tokio::spawn(VoiceLoop::new(config, port.clone()).run(
            connection,
            agent_rx,
            shutdown.clone(),
        ));
        Self {
            events: event_tx,
            commands: command_rx,
            agent: agent_tx,
            port,
            shutdown,
            task,
        }
    }

    fn quiet() -> Self {
        Self::start(VoiceChannelConfig {
            filler_after_ms: 0,
            ..Default::default()
        })
    }

    fn provider(&self, event: RealtimeEvent) {
        self.events.send(event).unwrap();
    }

    async fn agent(&self, output: AgentOutput) {
        self.agent.send(output).await.unwrap();
    }

    fn say(&self, transcript: &str) {
        self.provider(RealtimeEvent::InputTranscriptCompleted {
            item_id: "item".into(),
            transcript: transcript.into(),
        });
    }

    fn done(&self) {
        self.provider(RealtimeEvent::ResponseDone {
            response_id: "resp".into(),
            cancelled: false,
        });
    }

    async fn next_command(&mut self) -> RealtimeCommand {
        tokio::time::timeout(Duration::from_secs(5), self.commands.recv())
            .await
            .expect("command in time")
            .expect("command")
    }

    async fn next_spoken(&mut self) -> String {
        match self.next_command().await {
            RealtimeCommand::Speak { text } => text,
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    async fn settle(&self) {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    async fn finish(self) -> VoiceLoopSummary {
        self.shutdown.cancel();
        self.task.await.unwrap().unwrap()
    }
}

fn speak(text: &str) -> RealtimeCommand {
    RealtimeCommand::Speak { text: text.into() }
}

#[tokio::test]
async fn utterance_becomes_a_message_and_answer_is_spoken_sentence_by_sentence() {
    let mut h = CallRig::quiet();
    h.say("  What is the weather?  ");
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta("It is sunny. Highs".into()))
        .await;
    assert_eq!(h.next_spoken().await, "It is sunny.");
    h.agent(AgentOutput::TextDelta(" of 20 today.".into()))
        .await;
    h.agent(AgentOutput::TurnEnded).await;
    h.settle().await;
    // The second sentence waits for the first spoken response to finish.
    assert!(h.commands.try_recv().is_err());
    h.done();
    assert_eq!(h.next_spoken().await, "Highs of 20 today.");
    assert_eq!(h.port.sends(), vec!["What is the weather?".to_string()]);
    let calls = h.port.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, PortCall::Event(VoiceLoopEvent::AnswerStarted { .. })))
    );
    let summary = h.finish().await;
    assert_eq!(summary.utterances, 1);
    assert_eq!(summary.spoken_chunks, 2);
}

#[tokio::test]
async fn blank_transcripts_are_recorded_but_not_sent() {
    let h = CallRig::quiet();
    h.say("   ");
    h.settle().await;
    assert!(h.port.sends().is_empty());
    assert!(matches!(
        h.port.calls().as_slice(),
        [PortCall::Event(
            VoiceLoopEvent::InputTranscriptCompleted { .. }
        )]
    ));
    h.finish().await;
}

#[tokio::test]
async fn greeting_is_spoken_first_and_is_not_an_answer() {
    let mut h = CallRig::start(VoiceChannelConfig {
        greeting: Some("Hi, you are talking to an AI assistant.".into()),
        filler_after_ms: 0,
        ..Default::default()
    });
    assert_eq!(
        h.next_spoken().await,
        "Hi, you are talking to an AI assistant."
    );
    h.settle().await;
    assert!(
        !h.port
            .calls()
            .iter()
            .any(|c| matches!(c, PortCall::Event(VoiceLoopEvent::AnswerStarted { .. })))
    );
    h.finish().await;
}

#[tokio::test(start_paused = true)]
async fn filler_plays_once_when_the_agent_is_slow() {
    let mut h = CallRig::start(VoiceChannelConfig {
        filler_after_ms: 1_000,
        filler: "One moment.".into(),
        ..Default::default()
    });
    h.say("Book me a table.");
    h.settle().await;
    assert!(h.commands.try_recv().is_err());
    tokio::time::advance(Duration::from_millis(1_001)).await;
    assert_eq!(h.next_spoken().await, "One moment.");
    h.done();
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta("Done, booked for eight.".into()))
        .await;
    h.agent(AgentOutput::TurnEnded).await;
    assert_eq!(h.next_spoken().await, "Done, booked for eight.");
    h.done();
    tokio::time::advance(Duration::from_secs(5)).await;
    h.settle().await;
    assert!(h.commands.try_recv().is_err(), "filler plays only once");
    h.finish().await;
}

#[tokio::test(start_paused = true)]
async fn no_filler_when_the_answer_comes_first() {
    let mut h = CallRig::start(VoiceChannelConfig {
        filler_after_ms: 1_000,
        ..Default::default()
    });
    h.say("Hello?");
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta("Hello! ".into())).await;
    assert_eq!(h.next_spoken().await, "Hello!");
    h.done();
    tokio::time::advance(Duration::from_secs(5)).await;
    h.settle().await;
    assert!(h.commands.try_recv().is_err());
    h.finish().await;
}

#[tokio::test]
async fn barge_in_stops_speech_steers_and_mutes_the_stale_answer() {
    let mut h = CallRig::quiet();
    h.say("Tell me about Kyiv.");
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta(
        "Kyiv is the capital of Ukraine. It sits on the Dnipro. ".into(),
    ))
    .await;
    assert_eq!(h.next_spoken().await, "Kyiv is the capital of Ukraine.");
    h.provider(RealtimeEvent::OutputTranscriptDelta {
        response_id: "resp".into(),
        delta: "Kyiv is the".into(),
    });
    h.provider(RealtimeEvent::SpeechStarted);
    assert_eq!(h.next_command().await, RealtimeCommand::StopSpeaking);
    h.settle().await;
    assert_eq!(
        h.port.interruptions(),
        vec![VoiceLoopEvent::OutputInterrupted {
            heard: "Kyiv is the".into(),
            unspoken: "capital of Ukraine. It sits on the Dnipro.".into(),
            policy: Interruption::Steer,
        }]
    );
    assert!(!h.port.calls().contains(&PortCall::Cancel));

    // The old answer keeps streaming but is not spoken.
    h.agent(AgentOutput::TextDelta(
        "Population is about three million. ".into(),
    ))
    .await;
    h.settle().await;
    assert!(h.commands.try_recv().is_err());

    // The caller's next words steer; the new message is spoken.
    h.say("Actually, what about Lviv?");
    h.agent(AgentOutput::TextDelta("More stale text. ".into()))
        .await;
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta("Lviv is in the west. ".into()))
        .await;
    assert_eq!(h.next_spoken().await, "Lviv is in the west.");
    assert_eq!(
        h.port.sends(),
        vec![
            "Tell me about Kyiv.".to_string(),
            "Actually, what about Lviv?".to_string()
        ]
    );
    let summary = h.finish().await;
    assert_eq!(summary.interruptions, 1);
}

#[tokio::test]
async fn cancel_policy_cancels_the_turn_on_barge_in() {
    let mut h = CallRig::start(VoiceChannelConfig {
        interruption: Interruption::Cancel,
        filler_after_ms: 0,
        ..Default::default()
    });
    h.say("Read me the news.");
    h.agent(AgentOutput::MessageStarted).await;
    h.agent(AgentOutput::TextDelta("First headline. ".into()))
        .await;
    assert_eq!(h.next_spoken().await, "First headline.");
    h.provider(RealtimeEvent::SpeechStarted);
    assert_eq!(h.next_command().await, RealtimeCommand::StopSpeaking);
    h.settle().await;
    assert!(h.port.calls().contains(&PortCall::Cancel));
    h.finish().await;
}

#[tokio::test]
async fn caller_talking_while_nothing_is_spoken_is_not_a_barge_in() {
    let h = CallRig::quiet();
    h.say("One more thing.");
    h.provider(RealtimeEvent::SpeechStarted);
    h.settle().await;
    assert!(h.port.interruptions().is_empty());
    h.finish().await;
}

#[tokio::test]
async fn commentary_is_spoken_whole() {
    let mut h = CallRig::quiet();
    h.say("Check my calendar.");
    h.agent(AgentOutput::Commentary("Checking your calendar".into()))
        .await;
    assert_eq!(h.next_spoken().await, "Checking your calendar");
    h.finish().await;
}

#[tokio::test]
async fn provider_close_ends_the_loop() {
    let h = CallRig::quiet();
    let CallRig { events, task, .. } = h;
    drop(events);
    let summary = task.await.unwrap().unwrap();
    assert_eq!(summary, VoiceLoopSummary::default());
}

#[tokio::test]
async fn provider_errors_are_recorded_and_the_call_continues() {
    let mut h = CallRig::quiet();
    h.provider(RealtimeEvent::Error {
        message: "rate limited".into(),
    });
    h.say("Hi");
    h.agent(AgentOutput::Commentary("Hello there.".into()))
        .await;
    assert_eq!(h.next_command().await, speak("Hello there."));
    assert!(
        h.port
            .calls()
            .contains(&PortCall::Event(VoiceLoopEvent::ProviderError {
                message: "rate limited".into()
            }))
    );
    h.finish().await;
}

#[test]
fn chunks_cut_at_sentence_ends_and_keep_the_rest() {
    let mut buf = String::from("One. Two! Three? Four: five\nsix and sev");
    assert_eq!(
        take_chunks(&mut buf, false),
        vec!["One.", "Two!", "Three?", "Four:", "five"]
    );
    assert_eq!(buf, "six and sev");
    assert_eq!(take_chunks(&mut buf, true), vec!["six and sev"]);
    assert!(buf.is_empty());
}

#[test]
fn decimals_and_urls_do_not_split() {
    let mut buf = String::from("Pi is 3.14 and the site is example.com today");
    assert!(take_chunks(&mut buf, false).is_empty());
}

#[test]
fn long_text_without_punctuation_is_cut_at_a_word() {
    let mut buf = "word ".repeat(100);
    let chunks = take_chunks(&mut buf, false);
    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(|c| c.len() <= MAX_CHUNK_CHARS));
    assert!(chunks.iter().all(|c| !c.ends_with("wor")));
}

#[test]
fn markdown_is_stripped_for_speech() {
    assert_eq!(
        speakable("## Plan\n- **Fly** to `Kyiv`\n* rest").as_deref(),
        Some("Plan Fly to Kyiv rest")
    );
    assert_eq!(speakable("---\n| |"), None);
    assert_eq!(
        speakable("user_id is set").as_deref(),
        Some("user_id is set")
    );
}

#[test]
fn unspoken_tail_backs_up_to_the_word_start() {
    assert_eq!(
        unspoken_tail("Kyiv is the capital.", "Kyiv is th"),
        "the capital."
    );
    assert_eq!(unspoken_tail("Short.", "Short. extra"), "");
    assert_eq!(unspoken_tail("Nothing heard.", ""), "Nothing heard.");
}
