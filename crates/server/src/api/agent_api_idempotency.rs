// Safe retries on the Agent Execution API: `Idempotency-Key` on
// `POST …/sessions` and `POST …/sessions/{id}/messages`.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - Same store and contract as the command API (`command_dispatch`): a key is
//   remembered for a day, a retry with the same request replays the first
//   response with `Idempotent-Replayed: true`, a different request under the
//   same key is 422, and a retry while the first is still running is 409.
// - Keys belong to the caller (`ApiCaller::idempotency_owner`), so two
//   applications or two end users never collide on one key.
// - Only a success is remembered. A failure releases the key, so the client
//   can fix the request and retry with the same key.

use axum::{
    Json,
    body::{Bytes, to_bytes},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::storage::{EncryptionService, StorageBackend};

use super::channel_api::ChannelApiState;
use super::command_dispatch::{
    IDEMPOTENCY_KEY_TTL, IDEMPOTENCY_LOCK_TTL, IDEMPOTENT_REPLAYED_HEADER, idempotency_key,
    open_replay, request_fingerprint, seal_replay,
};
use crate::domains::agent_channels::api_sessions::ApiCaller;
use crate::domains::common::CommandError;
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
};

/// Largest response remembered for replay.
const MAX_REPLAY_BYTES: usize = 1024 * 1024;

/// A success as stored for replay.
#[derive(Serialize, Deserialize)]
struct StoredReply {
    status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    body: Value,
}

/// One retry-safe request: what it does (`operation`), to what (`target`),
/// and its body.
pub(crate) struct Retryable<'a> {
    pub org_id: i64,
    pub caller: &'a ApiCaller,
    pub operation: &'static str,
    pub target: String,
    pub body: &'a Bytes,
}

/// Run `handle` once per `Idempotency-Key`, or right away when the request
/// sent none.
pub(crate) async fn run_once(
    state: &ChannelApiState,
    headers: &HeaderMap,
    request: Retryable<'_>,
    handle: impl Future<Output = Response>,
) -> Response {
    let key = match idempotency_key(headers) {
        Ok(Some(key)) => key,
        Ok(None) => return handle.await,
        Err(err) => return failure(err),
    };
    let scope = IdempotencyKeyScope {
        org_id: request.org_id,
        principal_id: request.caller.idempotency_owner(),
        key,
    };
    let body: Value = serde_json::from_slice(request.body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(request.body).into_owned()));
    let fingerprint = request_fingerprint(
        request.operation,
        &serde_json::json!({ "target": request.target, "body": body }),
    );
    run_keyed(
        &state.db,
        state.encryption.as_ref(),
        scope,
        request.operation,
        fingerprint,
        handle,
    )
    .await
}

async fn run_keyed(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    scope: IdempotencyKeyScope,
    operation: &str,
    fingerprint: String,
    handle: impl Future<Output = Response>,
) -> Response {
    // The claim is part of admission, not a separately committed lease. A
    // process dying before its reply leaves either the operation and replay
    // together, or neither. The try-lock preserves an immediate 409 while
    // another request is alive, without blocking a pool slot on its lock.
    let attempt = async {
        if !db
            .try_lock_command_idempotency(&scope)
            .await
            .map_err(|err| failure(CommandError::internal(err)))?
        {
            return Err(in_progress());
        }
        let now = chrono::Utc::now();
        let claim = db
            .claim_command_idempotency_key(&ClaimIdempotencyKey {
                scope: scope.clone(),
                command: operation.to_string(),
                fingerprint: fingerprint.clone(),
                locked_until: now + IDEMPOTENCY_LOCK_TTL,
                expires_at: now + IDEMPOTENCY_KEY_TTL,
            })
            .await
            .map_err(|err| failure(CommandError::internal(err)))?;
        if let IdempotencyClaim::Existing(stored) = claim {
            if stored.fingerprint != fingerprint {
                return Err(failure(
                    CommandError::unprocessable(
                        "Idempotency-Key was already used for a different request; use a new key for a new request",
                    )
                    .with_code("idempotency_key_reused"),
                ));
            }
            return match stored.response {
                Some(sealed) => open_replay::<StoredReply>(encryption, &sealed)
                    .map(replay)
                    .map_err(failure),
                // An older server may still own a separately committed claim.
                // Keep its lease: clearing it during a rolling upgrade could
                // duplicate an operation that server has already accepted.
                None => Err(in_progress()),
            };
        }

        let response = handle.await;
        if !response.status().is_success() {
            return Err(response);
        }
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, MAX_REPLAY_BYTES).await.map_err(|err| {
            failure(CommandError::internal(anyhow::anyhow!(
                "agent API response too large to remember: {err}"
            )))
        })?;
        let reply = StoredReply {
            status: parts.status.as_u16(),
            location: parts
                .headers
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        };
        let sealed =
            seal_replay(encryption, &reply).map_err(|err| failure(CommandError::internal(err)))?;
        db.complete_command_idempotency_key(&scope, &sealed)
            .await
            .map_err(|err| failure(CommandError::internal(err)))?;
        Ok(Response::from_parts(parts, axum::body::Body::from(bytes)))
    };
    crate::storage::transaction::scope(db, Box::pin(attempt), |err| {
        failure(CommandError::internal(err))
    })
    .await
    .unwrap_or_else(|response| response)
}

fn in_progress() -> Response {
    failure(
        CommandError::conflict("A request with this Idempotency-Key is still running; retry later")
            .with_code("idempotency_key_in_progress"),
    )
}

fn replay(reply: StoredReply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    let mut response = (status, Json(reply.body)).into_response();
    let headers = response.headers_mut();
    headers.insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
    if let Some(location) = reply
        .location
        .and_then(|value| HeaderValue::from_str(&value).ok())
    {
        headers.insert(header::LOCATION, location);
    }
    response
}

fn failure(err: CommandError) -> Response {
    <(StatusCode, Json<_>)>::from(err).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateOrganizationRow, UpdateOrganization};
    use serde_json::json;
    use tokio::sync::Notify;
    use tokio::time::{Duration, timeout};

    async fn fixture() -> (Arc<StorageBackend>, IdempotencyKeyScope) {
        let db = Arc::new(StorageBackend::test_database());
        let org = db
            .create_organization(CreateOrganizationRow {
                public_id: format!("org_{}", uuid::Uuid::new_v4().simple()),
                name: "before".into(),
                created_by: None,
            })
            .await
            .unwrap();
        let scope = IdempotencyKeyScope {
            org_id: org.org_id,
            principal_id: uuid::Uuid::new_v4(),
            key: "retry-one-intent".into(),
        };
        (db, scope)
    }

    async fn effect(db: &StorageBackend, scope: &IdempotencyKeyScope) -> Response {
        db.update_organization(
            scope.org_id,
            UpdateOrganization {
                name: Some("after".into()),
            },
        )
        .await
        .unwrap()
        .expect("the effect's organization exists");
        (StatusCode::CREATED, Json(json!({"id": "stored-result"}))).into_response()
    }

    async fn name(db: &StorageBackend, scope: &IdempotencyKeyScope) -> String {
        db.get_organization(scope.org_id)
            .await
            .unwrap()
            .unwrap()
            .name
    }

    async fn retry(db: &StorageBackend, scope: &IdempotencyKeyScope) -> Response {
        timeout(Duration::from_secs(5), async {
            loop {
                let response = run_keyed(
                    db,
                    None,
                    scope.clone(),
                    "agent_api.test",
                    "same-request".into(),
                    effect(db, scope),
                )
                .await;
                if response.status() != StatusCode::CONFLICT {
                    break response;
                }
                // A cancelled task queues its connection's rollback before
                // handing it back. Give that rollback time to release its lock.
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("an aborted request must not strand a ten-minute claim")
    }

    async fn cancelled_request(after_effect: bool) {
        let (db, scope) = fixture().await;
        let entered = Arc::new(Notify::new());
        let owner = tokio::spawn({
            let db = db.clone();
            let scope = scope.clone();
            let entered = entered.clone();
            async move {
                run_keyed(
                    &db,
                    None,
                    scope.clone(),
                    "agent_api.test",
                    "same-request".into(),
                    async {
                        if after_effect {
                            let _ = effect(&db, &scope).await;
                        }
                        entered.notify_one();
                        std::future::pending::<Response>().await
                    },
                )
                .await
            }
        });
        timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        owner.abort();
        assert!(matches!(owner.await, Err(error) if error.is_cancelled()));
        assert_eq!(name(&db, &scope).await, "before");
        assert_eq!(retry(&db, &scope).await.status(), StatusCode::CREATED);
        assert_eq!(name(&db, &scope).await, "after");
        let replay = run_keyed(
            &db,
            None,
            scope,
            "agent_api.test",
            "same-request".into(),
            async { panic!("a committed retry must replay without another effect") },
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CREATED);
        assert_eq!(replay.headers()[IDEMPOTENT_REPLAYED_HEADER], "true");
        let bytes = to_bytes(replay.into_body(), MAX_REPLAY_BYTES)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["id"],
            "stored-result"
        );
    }

    #[tokio::test]
    async fn cancellation_before_admission_does_not_strand_a_claim() {
        cancelled_request(false).await;
    }

    #[tokio::test]
    async fn cancellation_after_the_effect_rolls_back_before_retry() {
        cancelled_request(true).await;
    }

    #[tokio::test]
    async fn a_concurrent_retry_gets_conflict_then_the_committed_response() {
        let (db, scope) = fixture().await;
        // The owner sweeps this expired row in its transaction. Another
        // caller's cleanup must skip that row instead of waiting for us.
        let mut expired = scope.clone();
        expired.key = "expired-key".into();
        let past = chrono::Utc::now() - chrono::Duration::seconds(1);
        db.claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: expired,
            command: "agent_api.test".into(),
            fingerprint: "old-request".into(),
            locked_until: past,
            expires_at: past,
        })
        .await
        .unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let owner = tokio::spawn({
            let db = db.clone();
            let scope = scope.clone();
            let entered = entered.clone();
            let release = release.clone();
            async move {
                run_keyed(
                    &db,
                    None,
                    scope.clone(),
                    "agent_api.test",
                    "same-request".into(),
                    async {
                        let response = effect(&db, &scope).await;
                        entered.notify_one();
                        release.notified().await;
                        response
                    },
                )
                .await
            }
        });
        timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        assert_eq!(name(&db, &scope).await, "before");
        let concurrent = timeout(
            Duration::from_secs(2),
            run_keyed(
                &db,
                None,
                scope.clone(),
                "agent_api.test",
                "same-request".into(),
                async { panic!("a contender cannot perform another effect") },
            ),
        )
        .await
        .expect("a contender must not block on the owner's transaction");
        assert_eq!(concurrent.status(), StatusCode::CONFLICT);
        let mut other_caller = scope.clone();
        other_caller.principal_id = uuid::Uuid::new_v4();
        let other = timeout(
            Duration::from_secs(5),
            run_keyed(
                &db,
                None,
                other_caller,
                "agent_api.test",
                "same-request".into(),
                async {
                    (StatusCode::CREATED, Json(json!({"id": "other-caller"}))).into_response()
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            other.status(),
            StatusCode::CREATED,
            "the lock is scoped to the caller"
        );
        release.notify_one();
        assert_eq!(owner.await.unwrap().status(), StatusCode::CREATED);
        assert_eq!(name(&db, &scope).await, "after");
        let replay = run_keyed(
            &db,
            None,
            scope,
            "agent_api.test",
            "same-request".into(),
            async { panic!("a completed operation must only replay") },
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CREATED);
        assert_eq!(replay.headers()[IDEMPOTENT_REPLAYED_HEADER], "true");
    }

    #[tokio::test]
    async fn failure_to_capture_the_reply_rolls_back_the_effect_and_claim() {
        let (db, scope) = fixture().await;
        let failed = run_keyed(
            &db,
            None,
            scope.clone(),
            "agent_api.test",
            "same-request".into(),
            async {
                let _ = effect(&db, &scope).await;
                Response::new(axum::body::Body::from(vec![b'x'; MAX_REPLAY_BYTES + 1]))
            },
        )
        .await;
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(name(&db, &scope).await, "before");
        assert_eq!(retry(&db, &scope).await.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn a_legacy_pending_claim_keeps_its_lease_during_a_rolling_upgrade() {
        let (db, scope) = fixture().await;
        let now = chrono::Utc::now();
        db.claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: scope.clone(),
            command: "agent_api.test".into(),
            fingerprint: "same-request".into(),
            locked_until: now + IDEMPOTENCY_LOCK_TTL,
            expires_at: now + IDEMPOTENCY_KEY_TTL,
        })
        .await
        .unwrap();
        let response = run_keyed(
            &db,
            None,
            scope,
            "agent_api.test",
            "same-request".into(),
            async { panic!("a legacy owner might still be accepting this input") },
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
    #[tokio::test]
    async fn a_lock_cannot_escape_to_an_implicit_or_different_transaction() {
        let (db, scope) = fixture().await;
        assert!(db.try_lock_command_idempotency(&scope).await.is_err());
        let (other, _) = fixture().await;
        crate::storage::transaction::scope(
            &other,
            async {
                assert!(db.try_lock_command_idempotency(&scope).await.is_err());
                Ok::<_, anyhow::Error>(())
            },
            |err| err,
        )
        .await
        .unwrap();
    }
    async fn input(
        db: &Arc<StorageBackend>,
        org: i64,
        session: everruns_contracts::typed_id::SessionId,
    ) -> Response {
        use crate::storage::CreateEventRow;
        use everruns_core::events::InputMessageData;
        let message = everruns_core::RuntimeMessage::user("one accepted intent");
        let id = message.id;
        let request = crate::turns::stored_message(
            session,
            crate::turns::scope(org, everruns_contracts::typed_id::HarnessId::new(), None),
            id,
            None,
        );
        let backend = crate::turns::ActorTurnBackend::new(db.clone(), None);
        crate::turns::store_input_and_start(
            db,
            &backend,
            db.create_event(CreateEventRow {
                session_id: session,
                event_type: "input.message".into(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data: serde_json::to_value(InputMessageData::new(message)).unwrap(),
                metadata: None,
                tags: None,
            }),
            request,
        )
        .await
        .unwrap();
        (StatusCode::CREATED, Json(json!({"id": id}))).into_response()
    }

    #[tokio::test]
    async fn cancelled_admission_rolls_back_the_input_and_actor_mark() {
        let (db, scope) = fixture().await;
        let session = db
            .create_session(crate::storage::test_database::test_session_row(
                scope.org_id,
            ))
            .await
            .unwrap()
            .id;
        let entered = Arc::new(Notify::new());
        let owner = tokio::spawn({
            let db = db.clone();
            let scope = scope.clone();
            let entered = entered.clone();
            async move {
                run_keyed(
                    &db,
                    None,
                    scope.clone(),
                    "agent_api.test",
                    "same-request".into(),
                    async {
                        let _ = input(&db, scope.org_id, session).await;
                        entered.notify_one();
                        std::future::pending::<Response>().await
                    },
                )
                .await
            }
        });
        timeout(Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        owner.abort();
        assert!(matches!(owner.await, Err(error) if error.is_cancelled()));
        assert!(
            db.list_message_events_limited(session, None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            db.get_session_actor(session)
                .await
                .unwrap()
                .is_none_or(|row| row.runnable_since.is_none())
        );
        let accepted = timeout(Duration::from_secs(5), async {
            loop {
                let response = run_keyed(
                    &db,
                    None,
                    scope.clone(),
                    "agent_api.test",
                    "same-request".into(),
                    input(&db, scope.org_id, session),
                )
                .await;
                if response.status() != StatusCode::CONFLICT {
                    break response;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(accepted.status(), StatusCode::CREATED);
        let inputs = db.list_message_events_limited(session, None).await.unwrap();
        assert_eq!(inputs.len(), 1);
        let actor = db.get_session_actor(session).await.unwrap().unwrap();
        assert_eq!(actor.inbox_seq, i64::from(inputs[0].sequence));
        let replay = run_keyed(
            &db,
            None,
            scope,
            "agent_api.test",
            "same-request".into(),
            async { panic!("a committed admission cannot append another input") },
        )
        .await;
        assert_eq!(replay.headers()[IDEMPOTENT_REPLAYED_HEADER], "true");
        assert_eq!(
            db.list_message_events_limited(session, None)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    #[tokio::test]
    async fn a_lock_cannot_fall_back_while_a_savepoint_owns_the_connection() {
        let (db, scope) = fixture().await;
        crate::storage::transaction::scope(
            &db,
            async {
                let _savepoint = db.tx_pool().begin().await.unwrap();
                assert!(db.try_lock_command_idempotency(&scope).await.is_err());
                Ok::<_, anyhow::Error>(())
            },
            |err| err,
        )
        .await
        .unwrap();
    }
}
