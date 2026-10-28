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
    let result = dispatch(&svc, &encryption, &remote, endpoint).await;
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
