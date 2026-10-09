//! Simulated speech-to-speech provider for tests and offline demos.
//!
//! A simulated call never touches the network. [`LlmSimRealtimeDriver`]
//! accepts any SDP offer and hands back a call id; the test then plays the
//! caller through [`SimulatedCall`], found by that id, and reads back
//! everything the voice loop asked the provider to say.
//!
//! Speaking is instant: every `Speak` command produces the matching output
//! transcript and a finished response, so a voice loop sees the same event
//! order it would from a real provider.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    BoxedRealtimeConnection, RealtimeCall, RealtimeCommand, RealtimeConnection, RealtimeDriver,
    RealtimeDriverError, RealtimeEvent, RealtimeSessionConfig,
};
use tokio::sync::{Notify, mpsc};

/// SDP answer returned for every simulated call.
pub const SIMULATED_ANSWER_SDP: &str = "v=0\r\no=llmsim 0 0 IN IP4 127.0.0.1\r\ns=llmsim\r\n";

/// The simulated realtime service of the `llmsim` provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct LlmSimRealtimeDriver;

#[derive(Default)]
struct CallSlot {
    caller: Option<mpsc::UnboundedSender<RealtimeEvent>>,
    events: Option<mpsc::UnboundedReceiver<RealtimeEvent>>,
    log: Arc<CallLog>,
    session: Option<RealtimeSessionConfig>,
}

#[derive(Default)]
struct CallLog {
    spoken: Mutex<Vec<String>>,
    stops: Mutex<u32>,
    changed: Notify,
    hung_up: std::sync::atomic::AtomicBool,
    hang_up: Notify,
}

fn calls() -> &'static Mutex<HashMap<String, CallSlot>> {
    static CALLS: OnceLock<Mutex<HashMap<String, CallSlot>>> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[async_trait]
impl RealtimeDriver for LlmSimRealtimeDriver {
    async fn accept_webrtc(
        &self,
        _endpoint: &ProviderEndpoint,
        session: &RealtimeSessionConfig,
        offer_sdp: &str,
    ) -> Result<RealtimeCall, RealtimeDriverError> {
        if offer_sdp.trim().is_empty() {
            return Err(RealtimeDriverError::Invalid("empty SDP offer".into()));
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let call_id = format!("rtc_llmsim_{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let (caller, events) = mpsc::unbounded_channel();
        lock(calls()).insert(
            call_id.clone(),
            CallSlot {
                caller: Some(caller),
                events: Some(events),
                log: Arc::default(),
                session: Some(session.clone()),
            },
        );
        Ok(RealtimeCall {
            answer_sdp: SIMULATED_ANSWER_SDP.to_string(),
            call_id: Some(call_id),
        })
    }

    async fn attach(
        &self,
        _endpoint: &ProviderEndpoint,
        call_id: &str,
    ) -> Result<BoxedRealtimeConnection, RealtimeDriverError> {
        let mut calls = lock(calls());
        let slot = calls
            .get_mut(call_id)
            .ok_or_else(|| RealtimeDriverError::Invalid(format!("unknown call {call_id}")))?;
        let events = slot
            .events
            .take()
            .ok_or_else(|| RealtimeDriverError::Invalid(format!("call {call_id} is attached")))?;
        Ok(Box::new(SimulatedConnection {
            events,
            pending: VecDeque::new(),
            log: slot.log.clone(),
            next_response: 0,
        }))
    }
}

struct SimulatedConnection {
    events: mpsc::UnboundedReceiver<RealtimeEvent>,
    /// Provider events produced by our own commands, delivered first.
    pending: VecDeque<RealtimeEvent>,
    log: Arc<CallLog>,
    next_response: u64,
}

#[async_trait]
impl RealtimeConnection for SimulatedConnection {
    async fn next_event(&mut self) -> Option<Result<RealtimeEvent, RealtimeDriverError>> {
        if let Some(event) = self.pending.pop_front() {
            return Some(Ok(event));
        }
        let hang_up = self.log.hang_up.notified();
        if self.log.hung_up.load(Ordering::SeqCst) {
            return None;
        }
        tokio::select! {
            event = self.events.recv() => event.map(Ok),
            () = hang_up => None,
        }
    }

    async fn send(&mut self, command: RealtimeCommand) -> Result<(), RealtimeDriverError> {
        match command {
            RealtimeCommand::Speak { text } => {
                self.next_response += 1;
                let response_id = format!("resp_llmsim_{}", self.next_response);
                lock(&self.log.spoken).push(text.clone());
                self.pending.extend([
                    RealtimeEvent::OutputTranscriptDelta {
                        response_id: response_id.clone(),
                        delta: text.clone(),
                    },
                    RealtimeEvent::OutputTranscriptCompleted {
                        response_id: response_id.clone(),
                        transcript: text,
                    },
                    RealtimeEvent::ResponseDone {
                        response_id,
                        cancelled: false,
                    },
                ]);
            }
            RealtimeCommand::StopSpeaking => {
                *lock(&self.log.stops) += 1;
            }
        }
        self.log.changed.notify_waiters();
        Ok(())
    }

    async fn close(&mut self) {
        self.events.close();
    }
}

/// The caller's side of a simulated call.
#[derive(Clone)]
pub struct SimulatedCall {
    call_id: String,
    caller: mpsc::UnboundedSender<RealtimeEvent>,
    log: Arc<CallLog>,
    session: Option<RealtimeSessionConfig>,
}

impl SimulatedCall {
    /// Find a call placed through [`LlmSimRealtimeDriver`].
    pub fn find(call_id: &str) -> Option<Self> {
        let calls = lock(calls());
        let slot = calls.get(call_id)?;
        Some(Self {
            call_id: call_id.to_string(),
            caller: slot.caller.clone()?,
            log: slot.log.clone(),
            session: slot.session.clone(),
        })
    }

    /// Provider call id.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Session settings the call was placed with.
    pub fn session(&self) -> Option<&RealtimeSessionConfig> {
        self.session.as_ref()
    }

    /// The caller says one utterance: speech starts, then the final transcript.
    pub fn say(&self, text: &str) {
        self.send(RealtimeEvent::SpeechStarted);
        self.send(RealtimeEvent::SpeechStopped);
        self.send(RealtimeEvent::InputTranscriptCompleted {
            item_id: format!("item_{}", text.len()),
            transcript: text.to_string(),
        });
    }

    /// Send any provider event, e.g. a bare `SpeechStarted` to barge in.
    pub fn send(&self, event: RealtimeEvent) {
        let _ = self.caller.send(event);
    }

    /// End the call from the provider side.
    pub fn hang_up(&self) {
        // The slot stays so a loop that attaches late sees the call end
        // instead of an unknown call; it is no longer findable.
        if let Some(slot) = lock(calls()).get_mut(&self.call_id) {
            slot.caller = None;
        }
        self.log.hung_up.store(true, Ordering::SeqCst);
        self.log.hang_up.notify_waiters();
    }

    /// Everything the provider was asked to say so far.
    pub fn spoken(&self) -> Vec<String> {
        lock(&self.log.spoken).clone()
    }

    /// How many times speech was stopped.
    pub fn stops(&self) -> u32 {
        *lock(&self.log.stops)
    }

    /// Wait until at least `count` lines were spoken, or `timeout` passes.
    pub async fn wait_spoken(&self, count: usize, timeout: Duration) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let changed = self.log.changed.notified();
            let spoken = self.spoken();
            if spoken.len() >= count {
                return spoken;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return self.spoken();
            }
        }
    }
}

/// Name the simulator and add its simulated realtime (voice) service, so a
/// voice channel can be exercised end to end without a speech provider.
pub(crate) fn with_simulated_realtime(
    mut descriptor: everruns_contracts::driver_registry::DriverDescriptor,
) -> everruns_contracts::driver_registry::DriverDescriptor {
    descriptor.display_name = "LLM Simulator".into();
    let Some(chat) = descriptor.chat.clone() else {
        return descriptor;
    };
    descriptor
        .services
        .push(everruns_contracts::driver_registry::ServiceKind::Realtime);
    descriptor.provider = Some(Arc::new(move |config| {
        everruns_contracts::Provider::from_driver(config.provider.clone(), chat(config).into())
            .with_driver_id(everruns_contracts::driver_registry::DriverId::LlmSim)
            .with_realtime(LlmSimRealtimeDriver)
    }));
    descriptor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn speak_produces_transcripts_and_a_finished_response() {
        let driver = LlmSimRealtimeDriver;
        let session = RealtimeSessionConfig::from_channel(&Default::default());
        let call = driver
            .accept_webrtc(&ProviderEndpoint::default(), &session, "v=0")
            .await
            .unwrap();
        assert_eq!(call.answer_sdp, SIMULATED_ANSWER_SDP);
        let call_id = call.call_id.unwrap();
        let caller = SimulatedCall::find(&call_id).unwrap();
        let mut connection = driver
            .attach(&ProviderEndpoint::default(), &call_id)
            .await
            .unwrap();
        assert!(
            driver
                .attach(&ProviderEndpoint::default(), &call_id)
                .await
                .is_err(),
            "a call attaches once"
        );

        caller.say("hello");
        assert_eq!(
            connection.next_event().await.unwrap().unwrap(),
            RealtimeEvent::SpeechStarted
        );
        connection
            .send(RealtimeCommand::Speak { text: "Hi!".into() })
            .await
            .unwrap();
        let next = connection.next_event().await.unwrap().unwrap();
        assert!(
            matches!(next, RealtimeEvent::OutputTranscriptDelta { delta, .. } if delta == "Hi!")
        );
        assert_eq!(caller.spoken(), vec!["Hi!".to_string()]);
        caller.hang_up();
        assert!(SimulatedCall::find(&call_id).is_none());
    }

    #[tokio::test]
    async fn empty_offer_is_rejected() {
        let session = RealtimeSessionConfig::from_channel(&Default::default());
        let err = LlmSimRealtimeDriver
            .accept_webrtc(&ProviderEndpoint::default(), &session, " ")
            .await
            .unwrap_err();
        assert!(matches!(err, RealtimeDriverError::Invalid(_)));
    }
}
