use super::super::tests::fixture;
use super::*;
use async_trait::async_trait;
use everruns_core::{
    EgressResponse, EgressResult, EgressStreamResponse, InputMessageData, RuntimeMessage,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Remote {
    mode: &'static str,
    calls: AtomicUsize,
}
#[async_trait]
impl EgressService for Remote {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        unreachable!()
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert!(request.dns_pinning_required);
        assert!(!request.headers.contains_key("Authorization"));
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            request.headers["X-Everruns-Signature"],
            signature("endpoint-secret", &request.body)
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = if body["type"] == "verify" {
            json!({"proof": if self.mode == "bad-proof" { "00".repeat(32) } else { signature("endpoint-secret", body["challenge"].as_str().unwrap().as_bytes()) }}).to_string()
        } else {
            assert!(request.scope.is_some());
            let turn = body["invocation_id"]
                .as_str()
                .unwrap()
                .replace("message_", "turn_");
            let mut events = vec![
                json!({"type":"turn.started", "data":{"turn_id":turn,"input_message_id":body["invocation_id"]}}),
            ];
            if self.mode == "scalar" {
                return Ok(EgressStreamResponse {
                    status: 200,
                    headers: Default::default(),
                    body: Box::pin(futures::stream::iter(vec![Ok(b"[]\n".to_vec())])),
                });
            }
            if self.mode == "bad-context" {
                events[0]["context"] = json!(123);
            }
            if self.mode == "forged" {
                events.push(json!({"type":"input.message", "data":{"message":body["input"]}}));
            }
            if self.mode != "disconnect" {
                events.push(json!({"type":"output.message.completed", "data":{"message":RuntimeMessage::assistant("27278 cents, 2 orders")}}));
                events
                    .push(json!({"type":"turn.completed", "data":{"turn_id":turn,"iterations":1}}));
            }
            let mut response = events
                .into_iter()
                .map(|v| v.to_string() + "\n")
                .collect::<String>();
            if self.mode == "trailing" {
                response.push_str("garbage\n");
            }
            response
        };
        // Deliberately split arbitrary byte boundaries, as fetch streams do.
        let chunks = response
            .as_bytes()
            .chunks(7)
            .map(|c| Ok(c.to_vec()))
            .collect::<Vec<_>>();
        Ok(EgressStreamResponse {
            status: 200,
            headers: Default::default(),
            body: Box::pin(futures::stream::iter(chunks)),
        })
    }
}

#[tokio::test]
async fn endpoint_verification_requires_secret_proof_and_https() {
    let remote = Remote {
        mode: "good",
        calls: AtomicUsize::new(0),
    };
    assert!(
        verify_endpoint(&remote, "https://agent.example/", "endpoint-secret")
            .await
            .is_ok()
    );
    let remote = Remote {
        mode: "bad-proof",
        calls: AtomicUsize::new(0),
    };
    assert!(
        verify_endpoint(&remote, "https://agent.example/", "endpoint-secret")
            .await
            .is_err()
    );
    for url in [
        "http://agent.example/",
        "https://user:pass@agent.example/",
        "https://agent.example/?token=secret",
        "https://agent.example/#token",
    ] {
        assert!(validate_endpoint(url).is_err());
    }
}

async fn exercise(mode: &'static str) -> (Vec<everruns_core::Event>, bool) {
    let (svc, deployment, session) = fixture().await;
    let user = RuntimeMessage::user("North sales?");
    svc.db
        .record_runtime_invocation(1, session, user.id.uuid(), None, None, None)
        .await
        .unwrap();
    let input = svc
        .events
        .emit(EventRequest::new(
            session,
            EventContext::default(),
            InputMessageData::new(user),
        ))
        .await
        .unwrap();
    if mode == "history-overflow" {
        for _ in 0..20 {
            svc.events
                .emit(EventRequest::new(
                    session,
                    EventContext::default(),
                    everruns_core::OutputMessageCompletedData::new(RuntimeMessage::assistant(
                        "x".repeat(500_000),
                    )),
                ))
                .await
                .unwrap();
        }
    }
    svc.db
        .mark_session_actor_runnable(session, 1, i64::from(input.sequence.unwrap()))
        .await
        .unwrap();
    if mode == "cancel" {
        sqlx::query("UPDATE session_actors SET cancel_requested_at = NOW(), cancel_inbox_seq = inbox_seq WHERE session_id = $1").bind(session).execute(svc.db.pool()).await.unwrap();
    }
    if mode == "interrupted" {
        sqlx::query("UPDATE session_actors SET attempts = 1 WHERE session_id = $1")
            .bind(session)
            .execute(svc.db.pool())
            .await
            .unwrap();
    }
    let encryption = EncryptionService::new(
        &crate::storage::encryption::generate_encryption_key("http-tests"),
        &[],
    )
    .unwrap();
    let endpoint = Endpoint {
        id: deployment,
        org_id: 1,
        endpoint_url: "https://agent.example/".into(),
        endpoint_secret_encrypted: encryption.encrypt_string("endpoint-secret").unwrap(),
        adapter_token_encrypted: encryption
            .encrypt_string(super::super::tests::TOKEN)
            .unwrap(),
    };
    let remote = Remote {
        mode,
        calls: AtomicUsize::new(0),
    };
    let result = if mode == "cancel-during" {
        dispatch(&svc, &encryption, &CancelDuring(svc.db.clone()), endpoint).await
    } else {
        dispatch(&svc, &encryption, &remote, endpoint).await
    };
    let rows = svc
        .db
        .read_session_actor_journal(session, Some(0), 100)
        .await
        .unwrap();
    let events = rows
        .events
        .into_iter()
        .map(EventService::row_to_event)
        .collect::<Vec<_>>();
    let row: (Option<String>, i64) =
        sqlx::query_as("SELECT lease_holder, consumed_seq FROM session_actors WHERE session_id=$1")
            .bind(session)
            .fetch_one(svc.db.pool())
            .await
            .unwrap();
    assert!(row.0.is_none());
    assert_eq!(row.1, i64::from(input.sequence.unwrap()));
    (events, result.is_ok())
}

#[tokio::test]
async fn streamed_execution_uses_canonical_events_and_releases_input() {
    let (events, success) = exercise("good").await;
    assert!(success);
    assert!(
        events
            .iter()
            .any(|e| e.event_type == "output.message.completed")
    );
    let terminal = events
        .iter()
        .find(|e| e.event_type == "turn.completed")
        .unwrap();
    assert!(terminal.context.trace_id.is_some());
    assert!(
        terminal
            .metadata
            .as_ref()
            .unwrap()
            .get("custom_worker_id")
            .is_some()
    );
}

#[tokio::test]
async fn disconnected_forged_or_trailing_stream_fails_once_without_replay() {
    for mode in [
        "disconnect",
        "forged",
        "trailing",
        "scalar",
        "bad-context",
        "interrupted",
        "history-overflow",
    ] {
        let (events, success) = exercise(mode).await;
        assert!(!success, "{mode}");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.event_type.as_str(), "turn.completed" | "turn.failed"))
                .count(),
            1,
            "{mode}"
        );
        assert!(events.iter().any(|e| e.event_type == "turn.failed"));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == "input.message")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn pending_cancel_is_terminal_without_invoking_external_code() {
    let (events, success) = exercise("cancel").await;
    assert!(!success);
    assert!(events.iter().any(|e| e.event_type == "turn.cancelled"));
    assert!(!events.iter().any(|e| e.event_type == "turn.completed"));
}

#[tokio::test]
async fn organization_admission_is_bounded_and_expiry_recovers_capacity() {
    let (svc, _, _) = fixture().await;
    for _ in 0..MAX_ORG_INVOCATIONS {
        assert!(admit(&svc.db, 1).await.unwrap().is_some());
    }
    assert!(admit(&svc.db, 1).await.unwrap().is_none());
    assert!(admit(&svc.db, 2).await.unwrap().is_some());
    sqlx::query("UPDATE external_worker_slots SET expires_at = NOW() - INTERVAL '1 second' WHERE org_id = 1").execute(svc.db.pool()).await.unwrap();
    assert!(admit(&svc.db, 1).await.unwrap().is_some());
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM external_worker_slots WHERE org_id = 1")
            .fetch_one(svc.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn slack_direct_reply_uses_the_same_external_worker_and_channel_delivery() {
    use crate::channels::slack::{
        events::SlackState,
        host_tests::{Slack, message, session_of, slack_config},
    };
    use everruns_core::channel::{
        ChannelDeliveryAdapter, DeliveryContext, DeliveryResult, OutboundChannelMessage,
        SessionBinding,
    };
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Delivery(Mutex<Vec<(SessionId, String)>>);
    #[async_trait]
    impl ChannelDeliveryAdapter for Delivery {
        fn platform(&self) -> &str {
            "slack"
        }
        async fn deliver(
            &self,
            message: &OutboundChannelMessage,
            _: &DeliveryContext,
        ) -> DeliveryResult {
            self.0
                .lock()
                .unwrap()
                .push((message.session_id, message.text.clone()));
            DeliveryResult::Ok
        }
        async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
            DeliveryResult::Ok
        }
    }
    let mut slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let delivery = Arc::new(Delivery::default());
    let runner = Arc::new(crate::turns::ActorTurnBackend::new(slack.db.clone(), None));
    let mut state = SlackState::new(
        slack.db.clone(),
        None,
        runner,
        crate::EventDelivery::in_memory(),
        "https://example.com/api".into(),
    );
    state
        .user_name_cache
        .insert("U1".into(), Some("Test person".into()));
    state.delivery_adapter = delivery.clone();
    slack.state = state.with_channel_host();
    let deployment: uuid::Uuid = sqlx::query_scalar("INSERT INTO custom_workers(org_id, agent_id, name, token_hash) VALUES(1,$1,'slack-http',$2) RETURNING id")
        .bind(slack.context.agent_internal_id).bind(Sha256::digest(super::super::tests::TOKEN.as_bytes()).as_slice()).fetch_one(slack.db.pool()).await.unwrap();
    let encryption = EncryptionService::new(
        &crate::storage::encryption::generate_encryption_key("slack-http"),
        &[],
    )
    .unwrap();
    let svc = CustomWorkerServiceImpl::new(slack.db.clone(), slack.state.event_service.clone());
    let remote = Remote {
        mode: "good",
        calls: AtomicUsize::new(0),
    };
    let mut session = None;
    for ts in ["1.0", "2.0"] {
        let mut body = message("C1", ts, if ts == "1.0" { None } else { Some("1.0") });
        body["event"]["text"] = json!("What are north sales?");
        let response = slack.send(body).await.unwrap();
        let current: SessionId = session_of(&response).parse().unwrap();
        if let Some(previous) = session {
            assert_eq!(previous, current);
        }
        session = Some(current);
        let endpoint = Endpoint {
            id: deployment,
            org_id: 1,
            endpoint_url: "https://agent.example/".into(),
            endpoint_secret_encrypted: encryption.encrypt_string("endpoint-secret").unwrap(),
            adapter_token_encrypted: encryption
                .encrypt_string(super::super::tests::TOKEN)
                .unwrap(),
        };
        dispatch(&svc, &encryption, &remote, endpoint)
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if delivery.0.lock().unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("CP did not deliver external replies to the Slack channel adapter");
    for (actual, text) in delivery.0.lock().unwrap().iter() {
        assert_eq!(Some(*actual), session);
        assert!(text.contains("27278"));
    }
}

struct CancelDuring(std::sync::Arc<StorageBackend>);
#[async_trait]
impl EgressService for CancelDuring {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        unreachable!()
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let session: SessionId = serde_json::from_value(body["session_id"].clone()).unwrap();
        let turn = body["invocation_id"]
            .as_str()
            .unwrap()
            .replace("message_", "turn_");
        let start = json!({"type":"turn.started","data":{"turn_id":turn,"input_message_id":body["invocation_id"]}}).to_string()+"\n";
        let db = self.0.clone();
        let stream = futures::stream::once(async move { Ok(start.into_bytes()) }).chain(
            futures::stream::once(async move {
                db.request_session_actor_cancel(session).await.unwrap();
                std::future::pending::<EgressResult<Vec<u8>>>().await
            }),
        );
        Ok(EgressStreamResponse {
            status: 200,
            headers: Default::default(),
            body: Box::pin(stream),
        })
    }
}

#[tokio::test]
async fn cancellation_during_a_stream_closes_execution_and_releases_the_input() {
    let (events, success) = exercise("cancel-during").await;
    assert!(!success);
    assert!(events.iter().any(|e| e.event_type == "turn.started"));
    assert!(events.iter().any(|e| e.event_type == "turn.cancelled"));
    assert!(!events.iter().any(|e| e.event_type == "turn.completed"));
}
