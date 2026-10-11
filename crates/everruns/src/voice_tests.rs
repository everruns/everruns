//! Unit tests for [`super`], the Framework voice channel, on the offline
//! simulated speech provider.

use std::time::Duration;

use super::*;
use crate::{Agent, InMemoryEngine, Model};

fn agent(reply: &str) -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated(reply))
        .build()
        .expect("agent builds")
}

#[tokio::test]
async fn a_call_speaks_the_greeting_and_the_agents_answer() {
    let session = InMemoryEngine::new().create(agent("It is sunny today."));
    let voice = VoiceChannel::delegated(Realtime::simulated())
        .voice("cedar")
        .greeting("Hi, you are talking to an AI assistant.");

    let call = voice
        .accept_webrtc(&session, "v=0")
        .await
        .expect("call placed");
    assert_eq!(
        call.answer_sdp(),
        everruns_llmsim::realtime::SIMULATED_ANSWER_SDP
    );
    let caller = SimulatedCall::find(call.call_id()).expect("simulated call");
    assert_eq!(caller.session().map(|s| s.voice.as_str()), Some("cedar"));
    let mut events = call.events();

    let spoken = caller.wait_spoken(1, Duration::from_secs(5)).await;
    assert_eq!(
        spoken,
        vec!["Hi, you are talking to an AI assistant.".to_string()]
    );

    caller.say("What is the weather?");
    let spoken = caller.wait_spoken(2, Duration::from_secs(10)).await;
    assert_eq!(
        spoken.get(1).map(String::as_str),
        Some("It is sunny today.")
    );

    // The utterance is an ordinary user message, marked as voice.
    let history = session.events_after(0).await.expect("history");
    let input = history
        .iter()
        .find(|event| event.event_type() == "input.message")
        .expect("user message recorded");
    let message = &input.canonical_json()["data"]["message"];
    assert_eq!(message["metadata"]["source"], "voice");

    // The caller's transcript reaches the call's event stream.
    let mut heard_input = false;
    while let Ok(event) = events.try_recv() {
        if matches!(event, VoiceCallEvent::InputTranscriptCompleted { ref transcript, .. } if transcript == "What is the weather?")
        {
            heard_input = true;
        }
    }
    assert!(heard_input, "input transcript reported");

    let summary = call.end().await.expect("call ends");
    assert_eq!(summary.utterances, 1);
}

#[tokio::test]
async fn hanging_up_ends_the_call() {
    let session = InMemoryEngine::new().create(agent("ok"));
    let call = VoiceChannel::delegated(Realtime::simulated())
        .accept_webrtc(&session, "v=0")
        .await
        .expect("call placed");
    SimulatedCall::find(call.call_id())
        .expect("simulated call")
        .hang_up();
    let summary = tokio::time::timeout(Duration::from_secs(5), call.wait())
        .await
        .expect("call ends when the caller hangs up")
        .expect("clean end");
    assert_eq!(summary.utterances, 0);
}

#[tokio::test]
async fn invalid_settings_and_empty_offers_are_rejected() {
    let session = InMemoryEngine::new().create(agent("ok"));
    let empty = VoiceChannel::delegated(Realtime::simulated())
        .accept_webrtc(&session, " ")
        .await
        .unwrap_err();
    assert!(matches!(empty, VoiceError::Invalid(_)), "got {empty:?}");

    let bad = VoiceChannel::delegated(Realtime::simulated())
        .voice("")
        .accept_webrtc(&session, "v=0")
        .await
        .unwrap_err();
    assert!(matches!(bad, VoiceError::Invalid(_)), "got {bad:?}");
}

#[test]
fn builder_sets_every_setting() {
    let channel = VoiceChannel::delegated(Realtime::simulated())
        .model("gpt-realtime-2")
        .voice("marin")
        .language("en")
        .greeting("Hello")
        .turn_detection(TurnDetection::SemanticVad)
        .interruption(Interruption::Cancel)
        .filler("Hold on.")
        .filler_after(Duration::from_millis(800))
        .speaking_style("Calm.");
    let config = channel.config();
    assert_eq!(config.language.as_deref(), Some("en"));
    assert_eq!(config.greeting.as_deref(), Some("Hello"));
    assert_eq!(config.turn_detection, TurnDetection::SemanticVad);
    assert_eq!(config.interruption, Interruption::Cancel);
    assert_eq!(config.filler, "Hold on.");
    assert_eq!(config.filler_after_ms, 800);
    assert_eq!(config.speaking_style.as_deref(), Some("Calm."));
}

async fn next_spoken(audio: &mut CallAudio) -> String {
    loop {
        let out = tokio::time::timeout(Duration::from_secs(10), audio.output.recv())
            .await
            .expect("speech in time")
            .expect("call still open");
        if let AudioOut::Audio(bytes) = out {
            return String::from_utf8(bytes)
                .expect("simulated speech is text")
                .trim_end_matches('\0')
                .to_string();
        }
    }
}

#[tokio::test]
async fn a_cascaded_call_hears_audio_and_speaks_the_answer() {
    let session = InMemoryEngine::new().create(agent("It is sunny today."));
    let voice = VoiceChannel::cascaded(Speech::simulated())
        .speech_model("tts-test")
        .greeting("Hello!");
    assert_eq!(voice.config().mode, VoiceMode::Cascaded);

    let (call, mut audio) = voice.accept_audio(&session).await.expect("call placed");
    assert!(call.call_id().starts_with("call_"));
    assert_eq!(call.answer_sdp(), "");
    assert_eq!(next_spoken(&mut audio).await, "Hello!");

    audio
        .input
        .send(b"What is the weather?".to_vec())
        .await
        .unwrap();
    assert_eq!(next_spoken(&mut audio).await, "It is sunny today.");

    let history = session.events_after(0).await.expect("history");
    let input = history
        .iter()
        .find(|event| event.event_type() == "input.message")
        .expect("user message recorded");
    assert_eq!(
        input.canonical_json()["data"]["message"]["metadata"]["source"],
        "voice"
    );

    // Hanging up is dropping the microphone.
    drop(audio.input);
    let summary = tokio::time::timeout(Duration::from_secs(10), call.wait())
        .await
        .expect("call ends")
        .expect("clean end");
    assert_eq!(summary.utterances, 1);
}

#[tokio::test]
async fn modes_and_transports_must_match() {
    let session = InMemoryEngine::new().create(agent("ok"));
    let cascaded = VoiceChannel::cascaded(Speech::simulated());
    assert!(matches!(
        cascaded.accept_webrtc(&session, "v=0").await,
        Err(VoiceError::Invalid(_))
    ));
    let delegated = VoiceChannel::delegated(Realtime::simulated());
    assert!(matches!(
        delegated.accept_audio(&session).await,
        Err(VoiceError::Invalid(_))
    ));
    // A delegated config on a cascaded provider is refused, not rewritten.
    let mismatched = VoiceChannel::with_config(Speech::simulated(), VoiceChannelConfig::default());
    assert!(matches!(
        mismatched.accept_audio(&session).await,
        Err(VoiceError::Invalid(why)) if why.contains("mode")
    ));
}
