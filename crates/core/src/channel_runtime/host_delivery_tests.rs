//! Scheduled delivery persistence and transaction boundaries.

use super::*;

#[tokio::test]
async fn scheduled_delivery_is_durable_before_send_returns() {
    let f = fixture(SessionBinding::Requester);
    let (session, _) = f
        .host
        .send_in_conversation(
            "support",
            "test:user:U1",
            InputMessage::user("briefing"),
            DeliveryTarget::new("D1", ""),
        )
        .await
        .unwrap();
    // No yield to the background delivery task: recovery must already be possible.
    let pending = f.store.pending_for_test().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].session_id, session.session_id);
    assert_eq!(pending[0].input_message_id, "turn_1");
}

// Models a transaction owner that retains background work until commit.
#[derive(Default)]
struct DeferredDeliveryStore {
    inner: MemoryChannelStore,
    tasks: Mutex<Vec<futures::future::BoxFuture<'static, ()>>>,
    fail_save: bool,
}

#[async_trait]
impl ChannelStore for DeferredDeliveryStore {
    fn spawn_delivery(&self, task: futures::future::BoxFuture<'static, ()>) {
        self.tasks.lock().unwrap().push(task);
    }
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError> {
        self.inner.session_for(channel, key).await
    }
    async fn bind(&self, channel: &str, key: &str, session: &str) -> Result<String, ChannelError> {
        self.inner.bind(channel, key, session).await
    }
    async fn first_sighting(&self, channel: &str, key: &str) -> Result<bool, ChannelError> {
        self.inner.first_sighting(channel, key).await
    }
    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        if self.fail_save {
            return Err(ChannelError::BadRequest(
                "injected pending write failure".into(),
            ));
        }
        self.inner.save_pending(pending, holder, lease).await
    }
    async fn clear_pending(&self, session: &str, input: &str) -> Result<(), ChannelError> {
        self.inner.clear_pending(session, input).await
    }
    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError> {
        self.inner.claim_pending(holder, lease).await
    }
}

#[tokio::test]
async fn scheduled_delivery_waits_for_commit_and_recovers_without_original_task() {
    let port = Arc::new(FakePort::default());
    let driver = Arc::new(TestDriver::default());
    let store = Arc::new(DeferredDeliveryStore::default());
    let host = ChannelHost::builder(port.clone())
        .store(store.clone())
        .channel(ChannelConfig::new("support"), driver.clone())
        .build();
    let (session, _) = host
        .send_in_conversation(
            "support",
            "test:user:U1",
            InputMessage::user("briefing"),
            DeliveryTarget::new("D1", ""),
        )
        .await
        .unwrap();
    port.reply(&session.session_id, "turn_1", "briefing");
    tokio::task::yield_now().await;
    assert!(
        driver.posts().is_empty(),
        "delivery cannot run before commit"
    );
    // Crash after committing durable state, before starting the queued task.
    store.tasks.lock().unwrap().clear();
    store.inner.expire_leases_for_test();
    assert_eq!(host.recover().await.unwrap(), 1);
    for task in std::mem::take(&mut *store.tasks.lock().unwrap()) {
        tokio::spawn(task);
    }
    eventually("recovered reply", || {
        driver.posts() == vec![(String::new(), "briefing".into())]
    })
    .await;
}

#[tokio::test]
async fn scheduled_delivery_fails_if_recovery_record_cannot_be_saved() {
    let port = Arc::new(FakePort::default());
    let driver = Arc::new(TestDriver::default());
    let store = Arc::new(DeferredDeliveryStore {
        fail_save: true,
        ..Default::default()
    });
    let host = ChannelHost::builder(port)
        .store(store.clone())
        .channel(ChannelConfig::new("support"), driver.clone())
        .build();
    let result = host
        .send_in_conversation(
            "support",
            "test:user:U1",
            InputMessage::user("briefing"),
            DeliveryTarget::new("D1", ""),
        )
        .await;
    assert!(result.is_err(), "caller must not commit its dedup receipt");
    assert!(store.tasks.lock().unwrap().is_empty());
    assert!(driver.posts().is_empty());
}

#[derive(Default)]
struct SlowDelivery {
    started: tokio::sync::Notify,
    finish: tokio::sync::Notify,
}

#[async_trait]
impl ChannelDeliveryAdapter for SlowDelivery {
    fn platform(&self) -> &str {
        "test"
    }
    async fn deliver(&self, _: &OutboundChannelMessage, _: &DeliveryContext) -> DeliveryResult {
        self.started.notify_one();
        self.finish.notified().await;
        DeliveryResult::Ok
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for SlowDelivery {
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        TestDriver::default().receive(request).await
    }
}

#[tokio::test]
async fn provider_io_does_not_prevent_the_host_from_renewing_its_delivery_lease() {
    let port = Arc::new(FakePort::default());
    let store = Arc::new(MemoryChannelStore::new());
    let driver = Arc::new(SlowDelivery::default());
    let host = ChannelHost::builder(port.clone())
        .store(store.clone())
        .delivery_lease(Duration::from_millis(90))
        .channel(ChannelConfig::new("support"), driver.clone())
        .build();
    host.handle("support", &message("hello", "t1", "m1")).await;
    port.reply("session_1", "turn_1", "reply");
    tokio::time::timeout(Duration::from_secs(1), driver.started.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        store
            .claim_pending("another-process", Duration::from_secs(60))
            .await
            .unwrap()
            .is_empty()
    );
    driver.finish.notify_one();
    eventually("slow delivery to finish", || {
        futures::executor::block_on(store.pending_for_test())
            .unwrap()
            .is_empty()
    })
    .await;
}

#[tokio::test]
async fn the_host_only_completes_confirmed_output_and_never_replays_uncertain_output() {
    for outcome in [
        DeliveryResult::Ok,
        DeliveryResult::Unknown { confirmed: vec![] },
    ] {
        let port = Arc::new(FakePort::default());
        let store = Arc::new(MemoryChannelStore::new());
        let driver = Arc::new(TestDriver {
            checkpointed: true,
            outcome: Some(outcome.clone()),
            ..Default::default()
        });
        let host = ChannelHost::builder(port.clone())
            .store(store.clone())
            .channel(ChannelConfig::new("support"), driver.clone())
            .build();
        let response = host.handle("support", &message("hello", "t1", "m1")).await;
        let session = response.body["session_id"].as_str().unwrap();
        port.reply(session, "turn_1", "reply");
        eventually("the reply attempt", || driver.posts().len() == 1).await;
        if matches!(outcome, DeliveryResult::Ok) {
            eventually("confirmed delivery completion", || {
                futures::executor::block_on(store.pending_for_test())
                    .unwrap()
                    .is_empty()
            })
            .await;
        } else {
            eventually("the uncertain checkpoint", || {
                futures::executor::block_on(store.delivery_checkpoint(session, "turn_1"))
                    .unwrap()
                    .is_some_and(|checkpoint| !checkpoint.uncertain.is_empty())
            })
            .await;
            store.expire_leases_for_test();
            assert_eq!(host.recover().await.unwrap(), 1);
            tokio::task::yield_now().await;
            assert_eq!(driver.posts().len(), 1);
            assert_eq!(store.pending_for_test().await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn unavailable_channel_does_not_discard_a_durable_output_ledger() {
    let store = Arc::new(MemoryChannelStore::new());
    let pending = PendingDelivery {
        channel: "temporarily-unpublished".into(),
        session_id: "session_1".into(),
        input_message_id: "turn_1".into(),
        target: DeliveryTarget::new("C1", "1.0"),
        after_sequence: 0,
    };
    store
        .save_pending(&pending, "old-owner", DEFAULT_DELIVERY_LEASE)
        .await
        .unwrap();
    store
        .save_delivery_checkpoint(
            "session_1",
            "turn_1",
            "old-owner",
            &super::super::super::DeliveryCheckpoint::default(),
        )
        .await
        .unwrap();
    store.expire_leases_for_test();
    let host = ChannelHost::builder(Arc::new(FakePort::default()))
        .store(store.clone())
        .build();
    assert_eq!(host.recover().await.unwrap(), 0);
    assert_eq!(store.pending_for_test().await.unwrap(), [pending]);
    assert!(
        store
            .delivery_checkpoint("session_1", "turn_1")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn legacy_driver_respects_existing_ledgers_and_delivery_ownership() {
    for (resolution, has_ledger, other_owner) in [
        (
            SendResolution::Assigned {
                turn_id: None,
                steered: false,
            },
            true,
            false,
        ),
        (SendResolution::Rejected, true, false),
        (
            SendResolution::Assigned {
                turn_id: None,
                steered: false,
            },
            false,
            true,
        ),
    ] {
        let port = Arc::new(FakePort::default());
        port.state().resolution = Some(resolution);
        let store = Arc::new(MemoryChannelStore::new());
        let driver = Arc::new(TestDriver::default());
        let host = ChannelHost::builder(port)
            .store(store.clone())
            .channel(ChannelConfig::new("support"), driver.clone())
            .build();
        let pending = PendingDelivery {
            channel: "support".into(),
            session_id: "session_1".into(),
            input_message_id: "turn_1".into(),
            target: DeliveryTarget::new("C1", "1.0"),
            after_sequence: 0,
        };
        let holder = if other_owner {
            "another-host"
        } else {
            &host.inner.holder
        };
        store
            .save_pending(&pending, holder, DEFAULT_DELIVERY_LEASE)
            .await
            .unwrap();
        if has_ledger {
            store
                .save_delivery_checkpoint(
                    "session_1",
                    "turn_1",
                    holder,
                    &super::super::super::DeliveryCheckpoint::default(),
                )
                .await
                .unwrap();
        }
        let events = [
            json!({"type":OUTPUT_MESSAGE_COMPLETED,"sequence":1,"context":{"input_message_id":"turn_1"},"data":{"message":{"id":"out-1","role":"assistant","content":[{"type":"text","text":"must not replay"}]}}}),
            json!({"type":TURN_COMPLETED,"sequence":2,"context":{"input_message_id":"turn_1"},"data":{}}),
        ].iter().map(|envelope| DeliveryEvent::from_envelope(envelope).unwrap()).collect::<Vec<_>>();
        super::super::delivery_task::run_delivery(
            host.inner.clone(),
            Delivery {
                channel: host.inner.channels["support"].clone(),
                driver: driver.clone(),
                session_id: "session_1".into(),
                input_message_id: "turn_1".into(),
                target: pending.target.clone(),
            },
            futures::stream::iter(events).boxed(),
        )
        .await;
        assert!(driver.posts().is_empty());
        assert_eq!(store.pending_for_test().await.unwrap(), [pending]);
        assert_eq!(
            store
                .delivery_checkpoint("session_1", "turn_1")
                .await
                .unwrap()
                .is_some(),
            has_ledger
        );
    }
}
