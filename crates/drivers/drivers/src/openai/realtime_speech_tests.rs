use super::*;
use everruns_contracts::runtime_provider::BearerAuth;
use everruns_contracts::voice::{DEFAULT_SPEECH_MODEL, SpeechDriver, VoiceChannelConfig};
use tokio::net::TcpListener;

use crate::openai::OpenAISpeechDriver;

fn request() -> SpeechRequest {
    SpeechRequest {
        model: DEFAULT_SPEECH_MODEL.into(),
        voice: "coral".into(),
        text: " Hello, \"friend\".\nDon't add anything. ".into(),
        instructions: Some("Warm and calm.".into()),
        safety_identifier: Some("offline-fixture".into()),
    }
}

fn created(id: &str) -> Value {
    json!({"type":"response.created","response":{"id":id,"metadata":{"everruns":MARKER}}})
}

fn delta(id: &str, bytes: &[u8]) -> Value {
    json!({"type":"response.output_audio.delta","response_id":id,
        "delta":base64::engine::general_purpose::STANDARD.encode(bytes)})
}

fn done(id: &str, status: &str) -> Value {
    json!({"type":"response.done","response":{"id":id,"status":status}})
}

struct SpeechHandshake;

impl tokio_tungstenite::tungstenite::handshake::server::Callback for SpeechHandshake {
    fn on_request(
        self,
        req: &tokio_tungstenite::tungstenite::handshake::server::Request,
        response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        assert_eq!(req.uri().path(), "/v1/realtime");
        assert_eq!(req.uri().query(), Some("model=gpt-realtime-2.1-mini"));
        assert_eq!(req.headers()["authorization"], "Bearer offline-not-a-key");
        assert_eq!(req.headers()["openai-safety-identifier"], "offline-fixture");
        Ok(response)
    }
}

async fn fixture(events: Vec<Value>) -> (ProviderEndpoint, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ProviderEndpoint::from_parts(
        format!("http://{}/v1", listener.local_addr().unwrap()),
        BearerAuth::new("offline-not-a-key"),
    );
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(tcp, SpeechHandshake)
            .await
            .unwrap();
        let mut received = Vec::new();
        for _ in 0..2 {
            let Message::Text(body) = socket.next().await.unwrap().unwrap() else {
                panic!("expected event")
            };
            received.push(serde_json::from_str::<Value>(&body).unwrap());
        }
        for event in events {
            if socket
                .send(Message::Text(event.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
        // Wait for the client to close/cancel; no tokens or external traffic.
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Text(body) => received.push(serde_json::from_str(&body).unwrap()),
                Message::Close(_) => break,
                _ => {}
            }
        }
        received
    });
    (endpoint, task)
}

async fn drain(events: Vec<Value>) -> (Vec<Vec<u8>>, Vec<String>, Vec<Value>) {
    let (endpoint, task) = fixture(events).await;
    let mut audio = OpenAISpeechDriver::new()
        .synthesize(&endpoint, &request())
        .await
        .unwrap();
    let mut frames = Vec::new();
    let mut errors = Vec::new();
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(3), audio.recv())
        .await
        .unwrap()
    {
        match frame {
            Ok(bytes) => frames.push(bytes),
            Err(error) => errors.push(error.to_string()),
        }
    }
    (frames, errors, task.await.unwrap())
}

#[tokio::test]
async fn authenticated_realtime_stream_preserves_pcm_voice_style_and_text_intent() {
    let (frames, errors, sent) = drain(vec![
        created("r1"),
        delta("unrelated", &[99, 99]),
        done("unrelated", "completed"),
        delta("r1", &[1, 2, 3]),
        delta("r1", &[4, 5, 6]),
        done("r1", "completed"),
    ])
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(frames.concat(), vec![1, 2, 3, 4, 5, 6]);
    assert!(frames.iter().all(|frame| frame.len() % 2 == 0));
    assert_eq!(sent[0], session(&request()));
    assert_eq!(
        sent[0]["session"]["audio"]["output"]["format"],
        json!({"type":"audio/pcm","rate":24000})
    );
    assert_eq!(sent[0]["session"]["audio"]["output"]["voice"], "coral");
    assert_eq!(sent[0]["session"]["tools"], json!([]));
    assert!(sent[0]["session"]["audio"]["input"]["turn_detection"].is_null());
    assert_eq!(sent[1]["response"]["conversation"], "none");
    assert_eq!(sent[1]["response"]["input"], json!([]));
    let instructions = sent[1]["response"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("Speaking style: Warm and calm."));
    assert!(instructions.ends_with(&request().text));
}

#[tokio::test]
async fn unsuccessful_statuses_and_audio_free_or_partial_responses_are_errors() {
    for status in ["failed", "cancelled", "incomplete", "unexpected"] {
        let (_, errors, _) =
            drain(vec![created("r"), delta("r", &[1, 2]), done("r", status)]).await;
        assert_eq!(errors.len(), 1, "{status}");
    }
    for events in [
        vec![created("r"), done("r", "completed")],
        vec![created("r"), delta("r", &[1, 2, 3]), done("r", "completed")],
        vec![
            created("r"),
            json!({"type":"response.output_audio.delta","response_id":"r","delta":"%%%"}),
        ],
        vec![
            created("r"),
            json!({"type":"response.output_audio.delta","delta":"AQI="}),
        ],
        vec![
            created("r"),
            json!({"type":"response.output_audio.delta","response_id":"r","delta":42}),
        ],
        vec![
            created("r"),
            json!({"type":"error","error":{"message":"private fixture content"}}),
        ],
    ] {
        let (_, errors, _) = drain(events).await;
        assert_eq!(errors.len(), 1);
        assert!(!errors[0].contains("private fixture content"));
    }
}

#[tokio::test]
async fn dropping_playback_cancels_the_correlated_response() {
    let (endpoint, task) = fixture(vec![created("r"), delta("r", &[1, 2])]).await;
    let mut audio = OpenAISpeechDriver::new()
        .synthesize(&endpoint, &request())
        .await
        .unwrap();
    assert_eq!(audio.recv().await.unwrap().unwrap(), [1, 2]);
    drop(audio);
    let sent = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent[2], json!({"type":"response.cancel","response_id":"r"}));
}

#[tokio::test]
async fn premature_close_and_malformed_json_are_errors_not_success() {
    for body in [None, Some("{broken JSON")] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = ProviderEndpoint::from_parts(
            format!("http://{}", listener.local_addr().unwrap()),
            BearerAuth::new("offline"),
        );
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            for _ in 0..2 {
                socket.next().await.unwrap().unwrap();
            }
            if let Some(body) = body {
                socket.send(Message::Text(body.into())).await.unwrap();
            }
            let _ = socket.close(None).await;
        });
        let mut audio = OpenAISpeechDriver::new()
            .synthesize(&endpoint, &request())
            .await
            .unwrap();
        assert!(audio.recv().await.unwrap().is_err());
        assert!(audio.recv().await.is_none());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn explicit_speech_api_override_keeps_its_endpoint_and_body() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };
    let server = MockServer::start().await;
    let mut request = request();
    request.model = "gpt-4o-mini-tts-2025-12-15".into();
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .and(body_json(super::super::speech::speech_body(&request)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3, 4]))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = ProviderEndpoint::from_parts(server.uri(), BearerAuth::new("offline"));
    let mut audio = OpenAISpeechDriver::new()
        .synthesize(&endpoint, &request)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(frame) = audio.recv().await {
        bytes.extend(frame.unwrap());
    }
    assert_eq!(bytes, vec![1, 2, 3, 4]);
}

#[test]
fn default_changes_without_rewriting_persisted_or_other_provider_overrides() {
    let config = VoiceChannelConfig::default();
    assert_eq!(config.speech_model(), "gpt-realtime-2.1-mini");
    assert!(uses_realtime(config.speech_model()));
    for model in ["gpt-realtime-2.1", "gpt-realtime-2.1-mini-2026-07-20"] {
        assert!(uses_realtime(model));
    }
    for model in ["gpt-4o-mini-tts", "tts-1-hd", "custom-provider-model"] {
        let config = VoiceChannelConfig {
            speech_model: Some(model.into()),
            ..Default::default()
        };
        assert_eq!(config.speech_model(), model);
        assert!(!uses_realtime(model));
    }
}
