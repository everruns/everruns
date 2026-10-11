use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::voice::{SpeechAudio, SpeechDriver, TranscriptionStream};

use super::*;

/// Speech service double: the test plays the transcription side, and every
/// synthesis returns `frames` frames of one second of audio each.
struct FakeSpeech {
    caller: Mutex<Option<mpsc::Receiver<Result<RealtimeEvent, RealtimeDriverError>>>>,
    frames: usize,
    requests: Mutex<Vec<SpeechRequest>>,
}

#[async_trait]
impl SpeechDriver for FakeSpeech {
    async fn transcribe(
        &self,
        _endpoint: &ProviderEndpoint,
        _config: &TranscriptionConfig,
    ) -> Result<TranscriptionStream, RealtimeDriverError> {
        let (audio, mut frames) = mpsc::channel(8);
        tokio::spawn(async move { while frames.recv().await.is_some() {} });
        Ok(TranscriptionStream {
            audio,
            events: lock(&self.caller).take().expect("one call"),
        })
    }

    async fn synthesize(
        &self,
        _endpoint: &ProviderEndpoint,
        request: &SpeechRequest,
    ) -> Result<SpeechAudio, RealtimeDriverError> {
        lock(&self.requests).push(request.clone());
        let (tx, rx) = mpsc::channel(self.frames.max(1));
        for _ in 0..self.frames {
            tx.send(Ok(vec![0; AUDIO_BYTES_PER_SECOND as usize]))
                .await
                .unwrap();
        }
        Ok(rx)
    }
}

async fn call(
    frames: usize,
) -> (
    CascadedCall,
    mpsc::Sender<Result<RealtimeEvent, RealtimeDriverError>>,
    Arc<FakeSpeech>,
) {
    let (caller, events) = mpsc::channel(8);
    let speech = Arc::new(FakeSpeech {
        caller: Mutex::new(Some(events)),
        frames,
        requests: Mutex::default(),
    });
    let config = VoiceChannelConfig {
        speaking_style: Some(" Calm. ".into()),
        ..Default::default()
    };
    let call = connect(
        speech.clone(),
        ProviderEndpoint::default(),
        &config,
        Some("user-hash".into()),
    )
    .await
    .unwrap();
    (call, caller, speech)
}

async fn next(call: &mut CascadedCall) -> RealtimeEvent {
    call.connection.next_event().await.unwrap().unwrap()
}

#[tokio::test(start_paused = true)]
async fn speak_streams_audio_then_reports_the_response_done() {
    let (mut call, _caller, speech) = call(2).await;
    call.connection
        .send(RealtimeCommand::Speak {
            text: "Your room is ready now.".into(),
        })
        .await
        .unwrap();
    let mut heard = String::new();
    loop {
        match next(&mut call).await {
            RealtimeEvent::OutputTranscriptDelta { delta, .. } => heard.push_str(&delta),
            RealtimeEvent::OutputTranscriptCompleted { transcript, .. } => {
                assert_eq!(transcript, "Your room is ready now.");
            }
            RealtimeEvent::ResponseDone {
                response_id,
                cancelled,
            } => {
                assert_eq!(response_id, "speech_1");
                assert!(!cancelled);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(heard, "Your room is ready now.");
    let mut audio = 0;
    while let Ok(AudioOut::Audio(bytes)) = call.audio_out.try_recv() {
        audio += bytes.len();
    }
    assert_eq!(audio, 2 * AUDIO_BYTES_PER_SECOND as usize);
    let request = lock(&speech.requests)[0].clone();
    assert_eq!(request.model, "gpt-4o-mini-tts");
    assert_eq!(request.voice, "marin");
    assert_eq!(request.instructions.as_deref(), Some("Calm."));
    assert_eq!(request.safety_identifier.as_deref(), Some("user-hash"));
}

#[tokio::test(start_paused = true)]
async fn audio_is_paced_at_playback_speed() {
    let (mut call, _caller, _speech) = call(4).await;
    let started = Instant::now();
    call.connection
        .send(RealtimeCommand::Speak {
            text: "A long answer that takes four seconds to say.".into(),
        })
        .await
        .unwrap();
    while !matches!(next(&mut call).await, RealtimeEvent::ResponseDone { .. }) {}
    // Four seconds of audio, handed over with the lead to spare.
    let took = started.elapsed();
    assert!(took >= Duration::from_secs(4) - LEAD, "{took:?}");
    assert!(took < Duration::from_secs(4), "{took:?}");
}

#[tokio::test(start_paused = true)]
async fn stop_speaking_clears_audio_and_drops_the_stale_response() {
    let (mut call, caller, _speech) = call(10).await;
    call.connection
        .send(RealtimeCommand::Speak {
            text: "One two three four five six seven eight nine ten.".into(),
        })
        .await
        .unwrap();
    // Let some speech play.
    let RealtimeEvent::OutputTranscriptDelta { .. } = next(&mut call).await else {
        panic!("expected speech");
    };
    call.connection
        .send(RealtimeCommand::StopSpeaking)
        .await
        .unwrap();
    let mut saw_clear = false;
    while let Ok(out) = call.audio_out.try_recv() {
        saw_clear |= out == AudioOut::Clear;
    }
    assert!(saw_clear);
    // Nothing more from the stopped response; the caller's events still flow.
    caller.send(Ok(RealtimeEvent::SpeechStopped)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(next(&mut call).await, RealtimeEvent::SpeechStopped);
}

#[tokio::test(start_paused = true)]
async fn talking_over_the_tail_of_a_finished_response_clears_it() {
    let (mut call, caller, _speech) = call(2).await;
    call.connection
        .send(RealtimeCommand::Speak {
            text: "Done.".into(),
        })
        .await
        .unwrap();
    while !matches!(next(&mut call).await, RealtimeEvent::ResponseDone { .. }) {}
    // The tail is still playing when the caller starts talking.
    caller.send(Ok(RealtimeEvent::SpeechStarted)).await.unwrap();
    assert_eq!(next(&mut call).await, RealtimeEvent::SpeechStarted);
    let mut last = None;
    while let Ok(out) = call.audio_out.try_recv() {
        last = Some(out);
    }
    assert_eq!(last, Some(AudioOut::Clear));
}

#[tokio::test]
async fn caller_hanging_up_ends_the_connection() {
    let (mut call, caller, _speech) = call(1).await;
    drop(caller);
    assert!(call.connection.next_event().await.is_none());
}

#[test]
fn heard_estimate_cuts_at_word_boundaries() {
    let mut heard = HeardEstimate::new("Hello there friend");
    assert_eq!(heard.advance(3), None);
    assert_eq!(heard.advance(8).as_deref(), Some("Hello"));
    assert_eq!(heard.advance(usize::MAX).as_deref(), Some(" there friend"));
    assert_eq!(heard.advance(usize::MAX), None);
}
