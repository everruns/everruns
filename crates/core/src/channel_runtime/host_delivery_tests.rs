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
    let pending = f
        .store
        .claim_pending("test", DEFAULT_DELIVERY_LEASE)
        .await
        .unwrap();
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
