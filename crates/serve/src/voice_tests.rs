//! The voice channel over a real socket, on the offline speech simulator. A
//! child of `wire_tests`, so it shares its server and test agents.

use everruns::voice::{SimulatedCall, VoiceChannelConfig};

use super::*;
use crate::config::AppConfig;
use crate::voice::{Speech, speech};

/// Answers every utterance with the weather, with no tools.
#[agent]
fn talker() -> Agent {
    Agent::builder()
        .model("sim")
        .description("Tells the weather.")
        .instructions("Answer briefly.")
        .tools(Vec::<String>::new())
        .offline(sim::script([sim::reply("It is sunny today.")]))
        .build()
}

/// The test app with `[voice]` on the simulator and a greeting.
fn voice_app() -> App {
    let config = AppConfig {
        voice: Some(VoiceChannelConfig {
            model: "sim".into(),
            voice: "cedar".into(),
            greeting: Some("Hi, you are talking to an AI assistant.".into()),
            ..VoiceChannelConfig::default()
        }),
        ..AppConfig::default()
    };
    let app = App::builder().discover().config(config).build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    app
}

#[tokio::test]
async fn a_call_greets_hears_the_caller_and_speaks_the_answer() {
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host.clone()).await;

    let response = server
        .post("/v1/channels/talker/voice/calls", json!({ "sdp": "v=0" }))
        .await;
    assert_eq!(response.status(), 200);
    let placed: Value = response.json().await.unwrap();
    assert_eq!(placed["voice"], "cedar");
    assert!(
        placed["answer_sdp"].as_str().unwrap().starts_with("v=0"),
        "{placed}"
    );
    let call_id = placed["call_id"].as_str().unwrap().to_string();
    let session_id = placed["session_id"].as_str().unwrap().to_string();

    let caller = SimulatedCall::find(&call_id).expect("simulated call");
    let spoken = caller.wait_spoken(1, Duration::from_secs(5)).await;
    assert_eq!(spoken, vec!["Hi, you are talking to an AI assistant."]);
    caller.say("What is the weather?");
    let spoken = caller.wait_spoken(2, Duration::from_secs(10)).await;
    assert_eq!(
        spoken.get(1).map(String::as_str),
        Some("It is sunny today.")
    );

    // The call's session is an ordinary session of the agent's voice channel,
    // with the utterance as a user message marked as voice.
    let session = server.session(&session_id).await;
    assert_eq!(session["agent_name"], "talker", "{session}");
    assert_eq!(session["metadata"]["kind"], "voice", "{session}");
    assert_eq!(session["metadata"]["channel"], "voice:talker", "{session}");
    let events = host.events_after(&session_id, 0).await.unwrap();
    let input = events
        .iter()
        .find(|event| event.event_type() == "input.message")
        .expect("the utterance is a message");
    assert_eq!(
        input.canonical_json()["data"]["message"]["metadata"]["source"],
        "voice"
    );

    let ended = server
        .post(
            &format!("/v1/channels/talker/voice/calls/{call_id}/end"),
            json!({}),
        )
        .await;
    assert_eq!(ended.status(), 200);
    let summary: Value = ended.json().await.unwrap();
    assert_eq!(summary["utterances"], 1, "{summary}");

    // Ended calls are gone.
    let again = server
        .post(
            &format!("/v1/channels/talker/voice/calls/{call_id}/end"),
            json!({}),
        )
        .await;
    assert_eq!(again.status(), 404);
}

#[tokio::test]
async fn a_call_can_continue_a_typed_session() {
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host.clone()).await;
    let session_id = host
        .create_session(NewSession {
            agent: Some("talker".into()),
            ..NewSession::default()
        })
        .await
        .unwrap();

    let placed: Value = server
        .post(
            "/v1/channels/talker/voice/calls",
            json!({ "sdp": "v=0", "session_id": session_id }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(placed["session_id"], session_id.as_str(), "{placed}");
    let call_id = placed["call_id"].as_str().unwrap();
    SimulatedCall::find(call_id).unwrap().hang_up();

    // Another agent's session is refused.
    let other = host
        .create_session(NewSession {
            agent: Some("asker".into()),
            ..NewSession::default()
        })
        .await
        .unwrap();
    let refused = server
        .post(
            "/v1/channels/talker/voice/calls",
            json!({ "sdp": "v=0", "session_id": other }),
        )
        .await;
    assert_eq!(refused.status(), 400);
}

#[tokio::test]
async fn bad_calls_are_refused() {
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;

    let empty = server
        .post("/v1/channels/talker/voice/calls", json!({ "sdp": " " }))
        .await;
    assert_eq!(empty.status(), 400);
    let unknown = server
        .post("/v1/channels/nobody/voice/calls", json!({ "sdp": "v=0" }))
        .await;
    assert_eq!(unknown.status(), 404);
    let no_session = server
        .post(
            "/v1/channels/talker/voice/calls",
            json!({ "sdp": "v=0", "session_id": "session_missing" }),
        )
        .await;
    assert_eq!(no_session.status(), 404);
    let no_call = server
        .post("/v1/channels/talker/voice/calls/rtc_none/end", json!({}))
        .await;
    assert_eq!(no_call.status(), 404);
}

#[tokio::test]
async fn invalid_voice_settings_stop_the_host() {
    let config = AppConfig {
        voice: Some(VoiceChannelConfig {
            voice: String::new(),
            ..VoiceChannelConfig::default()
        }),
        ..AppConfig::default()
    };
    let app = App::builder().discover().config(config).build();
    let err = Host::new(app, Mode::Eval, None).err().expect("refused");
    assert!(format!("{err:#}").contains("[voice]"), "{err:#}");
}

#[test]
fn speech_uses_openai_with_a_key_and_simulates_offline_only() {
    assert_eq!(speech("sim", Some("k"), false), Some(Speech::Simulated));
    assert_eq!(
        speech("gpt-realtime-2", Some("k"), false),
        Some(Speech::OpenAI("k".into()))
    );
    assert_eq!(
        speech("gpt-realtime-2", None, true),
        Some(Speech::Simulated)
    );
    assert_eq!(speech("gpt-realtime-2", None, false), None);
}

#[tokio::test]
async fn the_page_manifest_and_agent_card_list_the_channel() {
    let manifest = voice_app().manifest();
    for route in [
        "GET /v1/channels/talker/voice",
        "POST /v1/channels/talker/voice/calls",
        "POST /v1/channels/talker/voice/calls/{call_id}/end",
    ] {
        assert!(
            manifest.routes.contains(&route.to_string()),
            "{route}: {:?}",
            manifest.routes
        );
    }
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let card: Value = server.get("/v1/agent").await.json().await.unwrap();
    assert_eq!(card["voice"]["voice"], "cedar");
    assert_eq!(
        card["voice"]["endpoints"]["talker"],
        "/v1/channels/talker/voice"
    );

    let page = server.get("/v1/channels/talker/voice").await;
    assert_eq!(page.status(), 200);
    let html = page.text().await.unwrap();
    assert!(html.contains("<title>Talk to talker</title>"));
    assert_eq!(server.get("/v1/channels/nobody/voice").await.status(), 404);
}

/// The test app with a cascaded `[voice]` on the simulator.
fn cascaded_app() -> App {
    let config = AppConfig {
        voice: Some(VoiceChannelConfig {
            mode: everruns::voice::VoiceMode::Cascaded,
            speech_model: Some("sim".into()),
            greeting: Some("Hello!".into()),
            ..VoiceChannelConfig::default()
        }),
        ..AppConfig::default()
    };
    let app = App::builder().discover().config(config).build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    app
}

#[tokio::test]
async fn a_cascaded_call_runs_over_a_websocket() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let host = Host::new(cascaded_app(), Mode::Eval, None).unwrap();
    let server = serve(host.clone()).await;
    let url = format!(
        "{}/v1/channels/talker/voice/ws",
        server.base.replacen("http://", "ws://", 1)
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    let next_text = async |socket: &mut tokio_tungstenite::WebSocketStream<_>| -> Value {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .expect("message in time")
                .expect("socket open")
                .unwrap();
            if let Message::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    };
    let next_audio = async |socket: &mut tokio_tungstenite::WebSocketStream<_>| -> String {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .expect("message in time")
                .expect("socket open")
                .unwrap();
            if let Message::Binary(audio) = message {
                return String::from_utf8(audio.to_vec())
                    .unwrap()
                    .trim_end_matches('\0')
                    .to_string();
            }
        }
    };

    let hello = next_text(&mut socket).await;
    assert_eq!(hello["type"], "call", "{hello}");
    assert_eq!(hello["sample_rate"], 24000);
    let session_id = hello["session_id"].as_str().unwrap().to_string();
    assert_eq!(next_audio(&mut socket).await, "Hello!");

    // Simulated audio is text: one frame, one utterance.
    socket
        .send(Message::Binary(b"What is the weather?".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(next_audio(&mut socket).await, "It is sunny today.");

    let session = server.session(&session_id).await;
    assert_eq!(session["metadata"]["kind"], "voice", "{session}");

    // WebRTC calls are refused on a cascaded app, with the right route.
    let refused = server
        .post("/v1/channels/talker/voice/calls", json!({ "sdp": "v=0" }))
        .await;
    assert_eq!(refused.status(), 400);
    let body: Value = refused.json().await.unwrap();
    assert!(body.to_string().contains("/voice/ws"), "{body}");

    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn a_delegated_app_refuses_websocket_calls() {
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let url = format!(
        "{}/v1/channels/talker/voice/ws",
        server.base.replacen("http://", "ws://", 1)
    );
    let err = tokio_tungstenite::connect_async(url).await.unwrap_err();
    assert!(err.to_string().contains("400"), "{err}");
}

#[tokio::test]
async fn calls_with_one_conversation_key_share_a_session() {
    let host = Host::new(voice_app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let placed: Value = server
            .post(
                "/v1/channels/talker/voice/calls",
                json!({ "sdp": "v=0", "conversation": "caller-42" }),
            )
            .await
            .json()
            .await
            .unwrap();
        sessions.push(placed["session_id"].as_str().unwrap().to_string());
        let call_id = placed["call_id"].as_str().unwrap();
        server
            .post(
                &format!("/v1/channels/talker/voice/calls/{call_id}/end"),
                json!({}),
            )
            .await;
    }
    assert_eq!(sessions[0], sessions[1]);

    // A call without a key gets its own session.
    let other: Value = server
        .post("/v1/channels/talker/voice/calls", json!({ "sdp": "v=0" }))
        .await
        .json()
        .await
        .unwrap();
    assert_ne!(other["session_id"], sessions[0]);

    let both = server
        .post(
            "/v1/channels/talker/voice/calls",
            json!({ "sdp": "v=0", "conversation": "c", "session_id": sessions[0] }),
        )
        .await;
    assert_eq!(both.status(), 400);
}
