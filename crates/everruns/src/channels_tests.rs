//! Unit tests for [`super`], Framework channels on an in-memory engine and
//! the offline simulated model.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::{InMemoryEngine, Model};

fn agent(reply: &str) -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated(reply))
        .build()
        .expect("agent builds")
}

/// The webhook's request format, with every reply recorded instead of posted.
#[derive(Clone, Default)]
struct Recorder {
    posts: Arc<Mutex<Vec<(String, String)>>>,
}

impl Recorder {
    fn posts(&self) -> Vec<(String, String)> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Recorder {
    fn platform(&self) -> &str {
        "recorder"
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.posts
            .lock()
            .unwrap()
            .push((message.thread_ref.clone(), message.text.clone()));
        DeliveryResult::Ok
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for Recorder {
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        Webhook::new().receive(request).await
    }
}

fn message(text: &str, thread: &str) -> ChannelRequest {
    ChannelRequest::json(&json!({"text": text, "thread": thread}))
}

async fn wait_for_posts(recorder: &Recorder, count: usize) -> Vec<(String, String)> {
    for _ in 0..400 {
        let posts = recorder.posts();
        if posts.len() >= count {
            return posts;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("expected {count} posts, got {:?}", recorder.posts());
}

#[tokio::test]
async fn a_message_gets_the_agents_reply_on_its_thread() {
    let engine = InMemoryEngine::new();
    let recorder = Recorder::default();
    let channels = Channels::builder(&engine)
        .channel(
            Channel::new("support", recorder.clone()),
            agent("Hello from the agent."),
        )
        .build();

    let response = channels.handle("support", &message("hi", "t1")).await;
    assert_eq!(response.status, 200, "{:?}", response.body);
    assert!(response.body["session_id"].is_string());

    let posts = wait_for_posts(&recorder, 1).await;
    assert_eq!(
        posts,
        vec![("t1".to_string(), "Hello from the agent.".to_string())]
    );
}

#[tokio::test]
async fn a_thread_keeps_one_session_across_messages() {
    let engine = InMemoryEngine::new();
    let recorder = Recorder::default();
    let channels = Channels::builder(&engine)
        .channel(Channel::new("support", recorder.clone()), agent("Noted."))
        .build();

    let first = channels.handle("support", &message("one", "t1")).await;
    wait_for_posts(&recorder, 1).await;
    let second = channels.handle("support", &message("two", "t1")).await;
    wait_for_posts(&recorder, 2).await;
    let other = channels.handle("support", &message("three", "t2")).await;
    wait_for_posts(&recorder, 3).await;

    assert_eq!(first.body["session_id"], second.body["session_id"]);
    assert_ne!(first.body["session_id"], other.body["session_id"]);

    let session_id: SessionId = first.body["session_id"].as_str().unwrap().parse().unwrap();
    let session = engine.resume(session_id).await.unwrap();
    let inputs: Vec<String> = session
        .history()
        .page()
        .await
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| message.role == crate::MessageRole::User)
        .map(|message| message.text())
        .collect();
    assert_eq!(inputs, vec!["one".to_string(), "two".to_string()]);
}

#[tokio::test]
async fn start_posts_a_proactive_conversation_to_its_target() {
    let engine = InMemoryEngine::new();
    let recorder = Recorder::default();
    let channels = Channels::builder(&engine)
        .channel(
            Channel::new("digest", recorder.clone()).stream(false),
            agent("Today: all green."),
        )
        .build();

    channels
        .start(
            "digest",
            DeliveryTarget::new("ops", "daily"),
            "Post the daily digest",
        )
        .await
        .unwrap();
    let posts = wait_for_posts(&recorder, 1).await;
    assert_eq!(
        posts,
        vec![("daily".to_string(), "Today: all green.".to_string())]
    );
}

#[tokio::test]
async fn bad_requests_and_unknown_channels_are_answered_not_raised() {
    let engine = InMemoryEngine::new();
    let channels = Channels::builder(&engine)
        .channel(Channel::new("support", Recorder::default()), agent("x"))
        .build();
    assert_eq!(channels.names(), vec!["support".to_string()]);
    assert_eq!(
        channels
            .handle("support", &ChannelRequest::json(&json!({})))
            .await
            .status,
        400
    );
    assert_eq!(
        channels.handle("nope", &message("hi", "t1")).await.status,
        404
    );
}
