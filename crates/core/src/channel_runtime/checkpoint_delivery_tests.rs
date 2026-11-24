use super::*;
use crate::channel_runtime::{ChannelStore, MemoryChannelStore, PendingDelivery};
use std::time::Duration;

async fn checkpointed(recorder: &Arc<Recorder>) -> (TurnDelivery, Arc<MemoryChannelStore>) {
    let mut delivery = delivery(recorder, DeliveryOptions::default());
    delivery.context.auth_token = "must-never-be-checkpointed".into();
    let store = Arc::new(MemoryChannelStore::new());
    store
        .save_pending(
            &PendingDelivery {
                channel: "installation-a".into(),
                session_id: delivery.session_id.to_string(),
                input_message_id: TURN.into(),
                target: DeliveryTarget::new("C1", "t1"),
                after_sequence: 0,
            },
            "owner-a",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    delivery
        .enable_checkpoints(
            store.clone(),
            "owner-a".into(),
            &delivery.session_id.to_string(),
        )
        .await
        .unwrap();
    (delivery, store)
}

async fn restart(
    first: &TurnDelivery,
    recorder: &Arc<Recorder>,
    store: &Arc<MemoryChannelStore>,
) -> TurnDelivery {
    let mut next = TurnDelivery::new(
        recorder.clone(),
        first.context.clone(),
        first.session_id,
        TURN,
        DeliveryOptions::default(),
    );
    next.enable_checkpoints(
        store.clone(),
        "owner-a".into(),
        &first.session_id.to_string(),
    )
    .await
    .unwrap();
    next
}

fn durable(mut event: DeliveryEvent, sequence: i64) -> DeliveryEvent {
    event.sequence = Some(sequence);
    event
}

struct LegacyOpenError(Arc<Recorder>);

#[async_trait::async_trait]
impl ChannelDeliveryAdapter for LegacyOpenError {
    fn platform(&self) -> &str {
        "test"
    }
    fn requires_checkpoint(&self) -> bool {
        true
    }
    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        Some(self)
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.0.deliver(message, context).await
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait::async_trait]
impl ChannelStreamDelivery for LegacyOpenError {
    async fn start(&self, context: &DeliveryContext) -> Result<String, String> {
        self.0.start(context).await?;
        Err("synthetic-private-transport-detail".into())
    }
    async fn append(&self, handle: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        self.0.append(handle, text, context).await
    }
    async fn replace(&self, handle: &str, text: &str, context: &DeliveryContext) -> DeliveryResult {
        self.0.replace(handle, text, context).await
    }
    async fn stop(&self, handle: &str, context: &DeliveryContext) -> DeliveryResult {
        self.0.stop(handle, context).await
    }
}

#[tokio::test]
async fn legacy_string_failure_cannot_enable_a_blind_native_start_retry() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    first.adapter = Arc::new(LegacyOpenError(recorder.clone()));
    first.observe(&delta("out-1", "hello")).await;
    let saved = store
        .delivery_checkpoint(&first.session_id.to_string(), TURN)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("synthetic-private-transport-detail")
    );
    let mut next = restart(&first, &recorder, &store).await;
    next.adapter = Arc::new(LegacyOpenError(recorder.clone()));
    assert!(next.needs_reconciliation().await);
    next.observe(&completed("out-1", "hello")).await;
    next.observe(&event(TURN_COMPLETED, json!({}))).await;
    assert_eq!(recorder.calls(), [Call::Start]);
}

#[tokio::test]
async fn corrupt_stream_prefix_is_rejected_before_provider_io() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Default::default()
    });
    let (first, store) = checkpointed(&recorder).await;
    let mut checkpoint = crate::channel_runtime::DeliveryCheckpoint {
        state: first.snapshot(),
        ..Default::default()
    };
    checkpoint.state["streams"] = json!({"out-1":{"handle":"known","text":"é","sent":1}});
    store
        .save_delivery_checkpoint(&first.session_id.to_string(), TURN, "owner-a", &checkpoint)
        .await
        .unwrap();
    let mut next = delivery(&recorder, DeliveryOptions::default());
    assert!(
        next.enable_checkpoints(store, "owner-a".into(), &first.session_id.to_string())
            .await
            .is_err()
    );
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn every_split_message_link_survives_the_snapshot_to_link_commit_gap() {
    let recorder = Arc::new(Recorder::default());
    let (mut first, store) = checkpointed(&recorder).await;
    first.link("out-1", "C1:part-1".into());
    first.link("out-1", "C1:part-2".into());
    first.before_write("title", true).await.unwrap();
    first.after_write(&DeliveryResult::Ok).await;
    first.checkpoint().await;
    let mut next = restart(&first, &recorder, &store).await;
    assert_eq!(
        next.take_links(),
        [
            ("C1:part-1".into(), "out-1".into()),
            ("C1:part-2".into(), "out-1".into())
        ]
    );
}

#[tokio::test]
async fn acknowledged_reply_survives_restart_without_duplicate_post_or_credentials() {
    let recorder = Arc::new(Recorder::default());
    let (mut first, store) = checkpointed(&recorder).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    let saved = store
        .delivery_checkpoint(&first.session_id.to_string(), TURN)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("must-never-be-checkpointed")
    );
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(completed("out-1", "hello"), 2)).await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(recorder.calls(), [Call::Post("hello".into())]);
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn native_stream_restarts_at_its_confirmed_prefix_and_closes_the_same_handle() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    let prefix = "x".repeat(STREAM_FLUSH_CHARS);
    let mut chunk = delta("out-1", &prefix);
    chunk.sequence = None;
    first.observe(&chunk).await;
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(completed("out-1", &format!("{prefix}suffix")), 2))
        .await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(
        recorder.calls(),
        [
            Call::Start,
            Call::Append(prefix),
            Call::Append("suffix".into()),
            Call::Stop
        ]
    );
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn unknown_post_and_partial_acceptance_never_produce_a_second_post_or_notice() {
    for result in [
        DeliveryResult::Unknown { confirmed: vec![] },
        DeliveryResult::Partial {
            confirmed: vec!["C1:known-part".into()],
            cause: Box::new(DeliveryResult::RateLimited {
                retry_after: Duration::ZERO,
            }),
        },
    ] {
        let recorder = Arc::new(Recorder {
            reply_outcome: Mutex::new(Some(result)),
            ..Default::default()
        });
        let (mut first, store) = checkpointed(&recorder).await;
        first
            .observe(&durable(completed("out-1", "hello"), 2))
            .await;
        first
            .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
            .await;
        let mut next = restart(&first, &recorder, &store).await;
        assert!(next.needs_reconciliation().await);
        next.observe(&durable(completed("out-1", "hello"), 2)).await;
        next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
            .await;
        assert_eq!(recorder.calls(), [Call::Post("hello".into())]);
        assert!(next.needs_recovery().await);
    }
}

#[tokio::test]
async fn definitely_rate_limited_reply_can_resume_after_restart_without_fallback_notice() {
    let recorder = Arc::new(Recorder {
        reply_outcome: Mutex::new(Some(DeliveryResult::RateLimited {
            retry_after: Duration::ZERO,
        })),
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    first
        .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert!(first.needs_recovery().await);
    *recorder.reply_outcome.lock().unwrap() = None;
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(completed("out-1", "hello"), 2)).await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(
        recorder.calls(),
        [Call::Post("hello".into()), Call::Post("hello".into())]
    );
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn unknown_stream_start_cannot_fall_back_to_a_second_message() {
    let recorder = Arc::new(Recorder {
        stream: true,
        start_outcome: Mutex::new(Some(DeliveryResult::Unknown { confirmed: vec![] })),
        ..Default::default()
    });
    let (mut first, _) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "hello");
    chunk.sequence = None;
    first.observe(&chunk).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    first
        .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(recorder.calls(), [Call::Start]);
    assert!(first.needs_reconciliation().await);
}

#[tokio::test]
async fn refused_stream_append_keeps_the_handle_for_retry_and_never_resends_accepted_prefixes() {
    let recorder = Arc::new(Recorder {
        stream: true,
        append_outcome: Mutex::new(Some(DeliveryResult::RateLimited {
            retry_after: Duration::ZERO,
        })),
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "hello");
    chunk.sequence = None;
    first.observe(&chunk).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    first
        .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert!(!recorder.calls().contains(&Call::Stop));
    *recorder.append_outcome.lock().unwrap() = None;
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(completed("out-1", "hello"), 2)).await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(
        recorder
            .calls()
            .iter()
            .filter(|call| **call == Call::Start)
            .count(),
        1
    );
    assert_eq!(
        recorder
            .calls()
            .iter()
            .filter(|call| **call == Call::Stop)
            .count(),
        1
    );
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn crash_after_provider_acceptance_before_state_commit_is_not_replayed() {
    let recorder = Arc::new(Recorder::default());
    let (mut first, store) = checkpointed(&recorder).await;
    let owned_store = store.clone();
    *recorder.after_post.lock().unwrap() =
        Some(Arc::new(move || owned_store.expire_leases_for_test()));
    first
        .observe(&durable(completed("out-1", "accepted remotely"), 2))
        .await;
    assert!(first.needs_recovery().await);
    let session = first.session_id.to_string();
    let saved = store
        .delivery_checkpoint(&session, TURN)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.in_flight.as_ref().unwrap().confirmed, ["C1:accepted"]);
    assert_eq!(
        store
            .claim_pending("owner-b", Duration::from_secs(60))
            .await
            .unwrap()
            .len(),
        1
    );
    let mut next = TurnDelivery::new(
        recorder.clone(),
        first.context.clone(),
        first.session_id,
        TURN,
        DeliveryOptions::default(),
    );
    next.enable_checkpoints(store, "owner-b".into(), &first.session_id.to_string())
        .await
        .unwrap();
    assert!(next.needs_reconciliation().await);
    next.observe(&durable(completed("out-1", "accepted remotely"), 2))
        .await;
    assert_eq!(recorder.calls(), [Call::Post("accepted remotely".into())]);
}

#[tokio::test]
async fn a_rate_limited_stream_start_retries_the_start_instead_of_falling_back() {
    let recorder = Arc::new(Recorder {
        stream: true,
        start_outcome: Mutex::new(Some(DeliveryResult::RateLimited {
            retry_after: Duration::ZERO,
        })),
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "hello");
    chunk.sequence = None;
    first.observe(&chunk).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    first
        .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    *recorder.start_outcome.lock().unwrap() = None;
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(completed("out-1", "hello"), 2)).await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Post(_)))
    );
    assert_eq!(
        recorder
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Append(_)))
            .count(),
        1
    );
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn unknown_append_is_not_repeated_but_the_known_stream_is_closed() {
    let recorder = Arc::new(Recorder {
        stream: true,
        append_outcome: Mutex::new(Some(DeliveryResult::Unknown { confirmed: vec![] })),
        ..Default::default()
    });
    let (mut first, _) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "hello");
    chunk.sequence = None;
    first.observe(&chunk).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    first
        .observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(
        recorder.calls(),
        [Call::Start, Call::Append("hello".into()), Call::Stop]
    );
    assert!(first.needs_reconciliation().await);
}

#[tokio::test]
async fn guardrail_replacement_survives_an_unknown_edit_without_appending_old_text() {
    let recorder = Arc::new(Recorder {
        stream: true,
        replace_outcome: Mutex::new(Some(DeliveryResult::Unknown { confirmed: vec![] })),
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "preliminary");
    chunk.sequence = None;
    first.observe(&chunk).await;
    first.flush().await;
    first
        .observe(&durable(
            event(
                OUTPUT_MESSAGE_REPLACED,
                json!({"message_id":"out-1","replacement":"[retracted]"}),
            ),
            2,
        ))
        .await;
    let mut checkpoint = store
        .delivery_checkpoint(&first.session_id.to_string(), TURN)
        .await
        .unwrap()
        .unwrap();
    assert!(
        checkpoint
            .retries
            .keys()
            .any(|key| key.ends_with(":replace"))
    );
    for deadline in checkpoint.retries.values_mut() {
        *deadline = 0;
    }
    store
        .save_delivery_checkpoint(&first.session_id.to_string(), TURN, "owner-a", &checkpoint)
        .await
        .unwrap();
    *recorder.replace_outcome.lock().unwrap() = None;
    let mut next = restart(&first, &recorder, &store).await;
    next.observe(&durable(event(TURN_COMPLETED, json!({})), 3))
        .await;
    assert_eq!(
        recorder
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Append(_)))
            .count(),
        1
    );
    assert_eq!(
        recorder
            .calls()
            .iter()
            .filter(|call| **call == Call::Replace("[retracted]".into()))
            .count(),
        2
    );
    assert!(!next.needs_recovery().await);
}

#[tokio::test]
async fn recovered_start_receipt_closes_the_known_stream_without_replaying_unknown_text() {
    use crate::channel_runtime::{DeliveryAttempt, DeliveryCheckpoint};
    let recorder = Arc::new(Recorder {
        stream: true,
        reactions: true,
        ..Default::default()
    });
    let (first, store) = checkpointed(&recorder).await;
    let checkpoint = DeliveryCheckpoint {
        state: first.snapshot(),
        in_flight: Some(DeliveryAttempt {
            key: "reply:out-1:start".into(),
            confirmed: vec!["h1".into()],
            idempotent: false,
            outcome: None,
        }),
        ..Default::default()
    };
    store
        .save_delivery_checkpoint(&first.session_id.to_string(), TURN, "owner-a", &checkpoint)
        .await
        .unwrap();
    let mut next = restart(&first, &recorder, &store).await;
    assert_eq!(next.take_links(), [("C1:h1".into(), "out-1".into())]);
    assert!(next.needs_reconciliation().await);
    next.close_uncertain_streams().await;
    assert_eq!(recorder.calls(), [Call::Stop]);
    assert!(next.needs_reconciliation().await);
}

#[tokio::test]
async fn ephemeral_tokens_only_checkpoint_when_provider_output_changes() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    let mut chunk = delta("out-1", "a");
    chunk.sequence = None;
    first.observe(&chunk).await;
    let initial = store
        .delivery_checkpoint(&first.session_id.to_string(), TURN)
        .await
        .unwrap()
        .unwrap()
        .state;
    for _ in 0..50 {
        first.observe(&chunk).await;
    }
    assert_eq!(
        store
            .delivery_checkpoint(&first.session_id.to_string(), TURN)
            .await
            .unwrap()
            .unwrap()
            .state,
        initial
    );
    first.flush().await;
    assert_eq!(
        recorder.calls(),
        [Call::Start, Call::Append("a".repeat(51))]
    );
    assert_ne!(
        store
            .delivery_checkpoint(&first.session_id.to_string(), TURN)
            .await
            .unwrap()
            .unwrap()
            .state,
        initial
    );
}

#[tokio::test]
async fn committed_output_state_recreates_interrupted_message_link_writes() {
    let recorder = Arc::new(Recorder {
        reactions: true,
        ..Default::default()
    });
    let (mut first, store) = checkpointed(&recorder).await;
    first
        .observe(&durable(completed("out-1", "hello"), 2))
        .await;
    assert_eq!(first.take_links(), [("C1:posted".into(), "out-1".into())]);
    // The process stops before applying that separate link transaction.
    let mut next = restart(&first, &recorder, &store).await;
    assert_eq!(next.take_links(), [("C1:posted".into(), "out-1".into())]);
    next.observe(&durable(completed("out-1", "hello"), 2)).await;
    assert_eq!(recorder.calls(), [Call::Post("hello".into())]);
}
