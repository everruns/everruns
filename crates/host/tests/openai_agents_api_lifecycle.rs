//! Lifecycle of provider-held state for the OpenAI Agents API backend
//! (EVE-1126): stable outcomes when credentials, the preview, a model, or
//! the provider session become unavailable; credential rotation; and
//! idempotent remote deletion. Runs against the stateful fake API.
#![cfg(feature = "openai-agents-api")]

mod agents_api_support;

use agents_api_support::*;
use everruns_host::openai_agents_api::lifecycle::{ProviderDeletion, delete_provider_session};

fn failed(outcome: &AgentsApiTurnOutcome) -> (String, String) {
    match outcome {
        AgentsApiTurnOutcome::Failed {
            code,
            message,
            policy,
        } => {
            assert!(!policy, "a provider failure, not an Everruns policy stop");
            (code.clone().unwrap_or_default(), message.clone())
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
}

#[tokio::test]
async fn a_provider_session_deleted_out_of_band_fails_once_then_a_new_one_starts() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    let lost = h
        .checkpoint()
        .provider_session_id
        .expect("provider session");

    // The provider session disappears (deleted in the OpenAI dashboard,
    // expired by provider retention).
    h.fake.with(|s| s.sessions.clear());
    let second = request(2, "And customer 456?");
    let (outcome, crashes) = h.run(&second).await;
    assert_eq!(crashes, 0, "a stable outcome, not a retried error");
    let (code, message) = failed(&outcome);
    assert_eq!(code, "provider_session_unavailable");
    assert!(message.contains("new provider session"), "{message}");
    let checkpoint = h.checkpoint();
    assert_eq!(
        checkpoint.provider_session_id, None,
        "the lost session is released"
    );
    assert_eq!(checkpoint.provider_key.as_deref(), Some(PROVIDER_KEY));

    // A replay of the failed activity returns the same outcome without
    // calling the provider.
    let before = h.fake.with(|s| s.requests);
    let (replayed, _) = h.run(&second).await;
    assert_eq!(failed(&replayed).0, "provider_session_unavailable");
    assert_eq!(h.fake.with(|s| s.requests), before);

    // The next message starts a new provider session and completes.
    let (third, _) = h.run(&request(3, "Who is customer 123?")).await;
    assert_completed(&third);
    let replacement = h.checkpoint().provider_session_id.expect("new session");
    assert_ne!(replacement, lost);
    assert_eq!(h.fake.with(|s| s.creates), 2);
}

#[tokio::test]
async fn rejected_credentials_or_a_revoked_preview_end_the_turn_with_a_stable_code() {
    for status in [401, 403, 404] {
        let h = Harness::new().await;
        h.fake.with(|s| s.reject_all = Some(status));
        let (outcome, crashes) = h.run(&request(1, "hi")).await;
        assert_eq!(crashes, 0, "HTTP {status} is not retried");
        let (code, message) = failed(&outcome);
        assert_eq!(code, "provider_misconfigured", "HTTP {status}");
        assert!(
            !message.contains("sk-proj"),
            "a provider body that echoes the key never reaches the turn"
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn a_revoked_preview_on_an_existing_session_keeps_the_session() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    let session = h.checkpoint().provider_session_id;

    h.fake.with(|s| s.reject_all = Some(403));
    let (outcome, _) = h.run(&request(2, "And customer 456?")).await;
    assert_eq!(failed(&outcome).0, "provider_misconfigured");
    assert_eq!(
        h.checkpoint().provider_session_id,
        session,
        "access denied is not a lost session: restoring access resumes it"
    );

    h.fake.with(|s| s.reject_all = None);
    let (third, _) = h.run(&request(3, "Who is customer 123?")).await;
    assert_completed(&third);
    assert_eq!(h.fake.with(|s| s.creates), 1, "the same provider session");
}

#[tokio::test]
async fn an_unavailable_model_fails_the_turn_with_model_unavailable() {
    let h = Harness::new().await;
    h.fake.with(|s| s.model_unavailable = true);
    let (outcome, crashes) = h.run(&request(1, "hi")).await;
    assert_eq!(crashes, 0);
    assert_eq!(failed(&outcome).0, "model_unavailable");
    assert_eq!(h.checkpoint().provider_session_id, None);
}

#[tokio::test]
async fn a_rotated_key_keeps_the_provider_session_and_another_provider_starts_a_new_one() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    let original = h.checkpoint().provider_session_id.unwrap();

    // Same Everruns provider, rotated key: the session stays reachable.
    let rotated = AgentsApiTurnDriver::new(
        AgentsApiClient::new("rotated-key").with_base_url(h.server.uri()),
        h.store.clone(),
        h.ledger.clone(),
        h.executor.clone(),
    )
    .with_reconnect_policy(4, Duration::from_millis(1))
    .with_usage_poll(2, Duration::from_millis(1));
    let outcome = rotated
        .run(&request(2, "Who is customer 123?"))
        .await
        .unwrap();
    assert_completed(&outcome);
    assert_eq!(
        h.checkpoint().provider_session_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(h.fake.with(|s| s.creates), 1);

    // Another Everruns provider (another OpenAI project): a new session.
    let mut moved = request(3, "Who is customer 123?");
    moved.provider_key = Some("01933b5a-0000-7000-8000-000000000002".into());
    let (third, _) = h.run(&moved).await;
    assert_completed(&third);
    let checkpoint = h.checkpoint();
    assert_ne!(
        checkpoint.provider_session_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(
        checkpoint.provider_key.as_deref(),
        Some("01933b5a-0000-7000-8000-000000000002")
    );
    assert_eq!(h.fake.with(|s| s.creates), 2);
}

#[tokio::test]
async fn a_checkpoint_without_a_recorded_provider_adopts_the_turns_provider() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    // A checkpoint written before providers were recorded.
    let lease = AgentsApiLease {
        org_id: 1,
        session_id: SessionId::from_seed(1),
        owner: uuid::Uuid::new_v4(),
    };
    let mut legacy = h.store.acquire(lease).await.unwrap();
    legacy.provider_key = None;
    h.store.save(lease, &legacy).await.unwrap();
    h.store.release(lease).await.unwrap();

    let (second, _) = h.run(&request(2, "Who is customer 123?")).await;
    assert_completed(&second);
    assert_eq!(h.checkpoint().provider_key.as_deref(), Some(PROVIDER_KEY));
    assert_eq!(h.fake.with(|s| s.creates), 1, "the session is kept");
}

#[tokio::test]
async fn deleting_a_provider_session_is_idempotent() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    let session = h.checkpoint().provider_session_id.unwrap();
    let client = AgentsApiClient::new("test-key").with_base_url(h.server.uri());

    assert_eq!(
        delete_provider_session(&client, &session).await.unwrap(),
        ProviderDeletion::Deleted
    );
    assert_eq!(
        delete_provider_session(&client, &session).await.unwrap(),
        ProviderDeletion::AlreadyGone,
        "a retried delete converges"
    );
    assert_eq!(h.fake.with(|s| s.deletes.len()), 2);
    assert!(h.fake.with(|s| s.sessions.is_empty()));

    // Rejected credentials are an error to retry, not a deletion.
    h.fake.with(|s| s.reject_all = Some(401));
    assert!(matches!(
        delete_provider_session(&client, "sess_other").await,
        Err(AgentsApiError::Api { status: 401, .. })
    ));
}
