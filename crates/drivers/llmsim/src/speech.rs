//! Simulated speech-to-text and text-to-speech for cascaded voice tests and
//! offline demos.
//!
//! The "audio" is text: every caller frame [`LlmSimSpeechDriver`] transcribes
//! is read as UTF-8 and becomes one finished utterance, and synthesized speech
//! is the spoken text's bytes. A test then drives a whole cascaded call by
//! sending sentences and reading back what was said, with no network.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::voice::{
    RealtimeDriverError, RealtimeEvent, SpeechAudio, SpeechDriver, SpeechRequest,
    TranscriptionConfig, TranscriptionStream,
};
use tokio::sync::mpsc;

/// The simulated speech service of the `llmsim` provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct LlmSimSpeechDriver;

#[async_trait]
impl SpeechDriver for LlmSimSpeechDriver {
    async fn transcribe(
        &self,
        _endpoint: &ProviderEndpoint,
        _config: &TranscriptionConfig,
    ) -> Result<TranscriptionStream, RealtimeDriverError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let (audio, mut frames) = mpsc::channel::<Vec<u8>>(64);
        let (events_tx, events) = mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                let Ok(text) = String::from_utf8(frame) else {
                    continue;
                };
                let text = text.trim_matches(char::from(0)).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let item_id = format!("item_llmsim_{}", NEXT.fetch_add(1, Ordering::Relaxed));
                for event in [
                    RealtimeEvent::SpeechStarted,
                    RealtimeEvent::SpeechStopped,
                    RealtimeEvent::InputTranscriptCompleted {
                        item_id,
                        transcript: text,
                    },
                ] {
                    if events_tx.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(TranscriptionStream { audio, events })
    }

    async fn synthesize(
        &self,
        _endpoint: &ProviderEndpoint,
        request: &SpeechRequest,
    ) -> Result<SpeechAudio, RealtimeDriverError> {
        let mut bytes = request.text.clone().into_bytes();
        // Whole 16-bit samples, like real audio.
        if bytes.len() % 2 == 1 {
            bytes.push(0);
        }
        let (tx, rx) = mpsc::channel(1);
        let _ = tx.send(Ok(bytes)).await;
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::voice::VoiceChannelConfig;

    #[tokio::test]
    async fn each_frame_is_one_utterance_and_speech_is_the_text() {
        let driver = LlmSimSpeechDriver;
        let endpoint = ProviderEndpoint::default();
        let config = TranscriptionConfig::from_channel(&VoiceChannelConfig::default());
        let mut stream = driver.transcribe(&endpoint, &config).await.unwrap();
        stream.audio.send(b"Book a room".to_vec()).await.unwrap();
        assert_eq!(
            stream.events.recv().await.unwrap().unwrap(),
            RealtimeEvent::SpeechStarted
        );
        assert_eq!(
            stream.events.recv().await.unwrap().unwrap(),
            RealtimeEvent::SpeechStopped
        );
        let RealtimeEvent::InputTranscriptCompleted { transcript, .. } =
            stream.events.recv().await.unwrap().unwrap()
        else {
            panic!("expected a transcript");
        };
        assert_eq!(transcript, "Book a room");
        drop(stream.audio);
        assert!(stream.events.recv().await.is_none());

        let mut audio = driver
            .synthesize(
                &endpoint,
                &SpeechRequest {
                    model: "m".into(),
                    voice: "v".into(),
                    text: "Hi!".into(),
                    instructions: None,
                    safety_identifier: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(audio.recv().await.unwrap().unwrap(), b"Hi!\0".to_vec());
        assert!(audio.recv().await.is_none());
    }
}
