//! Host-owned output intents. A committed intent without a committed settlement
//! is uncertain, even when the process died before reaching the provider.
//! Reconciliation is explicit; this ledger never promises remote exactly-once.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use web_time::{SystemTime, UNIX_EPOCH};

use super::{ChannelError, ChannelStore, DeliveryResult};
use everruns_contracts::runtime::channel::DeliveryReceiptSink;

const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PARTS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryAttempt {
    pub key: String,
    pub confirmed: Vec<String>,
    /// Repeating a state replacement/cleanup does not create another output.
    pub idempotent: bool,
    pub outcome: Option<DeliveryResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryCheckpoint {
    pub version: u32,
    pub state: Value,
    pub in_flight: Option<DeliveryAttempt>,
    pub uncertain: Vec<DeliveryAttempt>,
    /// Absolute UTC milliseconds, retained through process restarts.
    pub retries: HashMap<String, u64>,
    /// Definite permanent refusals must not be dispatched again on event replay.
    #[serde(default)]
    pub refused: HashSet<String>,
}

impl Default for DeliveryCheckpoint {
    fn default() -> Self {
        Self {
            version: 1,
            state: Value::Null,
            in_flight: None,
            uncertain: Vec::new(),
            retries: HashMap::new(),
            refused: HashSet::new(),
        }
    }
}

/// Scoped by the host, never by provider input or an adapter-selected tenant.
pub(super) struct DeliveryJournal {
    store: Arc<dyn ChannelStore>,
    session: String,
    input: String,
    holder: String,
    checkpoint: Mutex<DeliveryCheckpoint>,
    failed: AtomicBool,
    dirty: AtomicBool,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| {
            u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
        })
}

impl DeliveryJournal {
    pub(super) fn new(
        store: Arc<dyn ChannelStore>,
        session: String,
        input: String,
        holder: String,
        mut checkpoint: DeliveryCheckpoint,
    ) -> Self {
        let recovering = checkpoint.in_flight.is_some();
        // No evidence distinguishes a crash before send from a crash after acceptance.
        if let Some(attempt) = checkpoint.in_flight.take() {
            if !attempt.idempotent {
                checkpoint.uncertain.push(attempt);
            } else if !matches!(attempt.key.as_str(), "status" | "title")
                && !attempt.key.starts_with("reaction:")
            {
                checkpoint.retries.insert(attempt.key, now_millis());
            }
        }
        Self {
            store,
            session,
            input,
            holder,
            checkpoint: Mutex::new(checkpoint),
            failed: AtomicBool::new(false),
            dirty: AtomicBool::new(recovering),
        }
    }

    async fn persist(&self, checkpoint: &DeliveryCheckpoint) -> Result<(), ChannelError> {
        let bytes = serde_json::to_vec(checkpoint)
            .map_err(|_| ChannelError::Session("invalid delivery checkpoint".into()))?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            self.failed.store(true, Ordering::SeqCst);
            return Err(ChannelError::Session(
                "delivery checkpoint capacity exceeded".into(),
            ));
        }
        let result = self
            .store
            .save_delivery_checkpoint(&self.session, &self.input, &self.holder, checkpoint)
            .await;
        if result.is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
        result
    }

    pub(super) fn has_changes(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    pub(super) async fn resolve_retries(&self, prefix: &str) {
        self.checkpoint
            .lock()
            .await
            .retries
            .retain(|key, _| !key.starts_with(prefix));
    }

    pub(super) async fn retrying(&self, key: &str) -> bool {
        self.checkpoint.lock().await.retries.contains_key(key)
    }

    pub(super) async fn blocked(&self) -> bool {
        self.failed.load(Ordering::SeqCst) || !self.checkpoint.lock().await.uncertain.is_empty()
    }

    pub(super) async fn pending(&self) -> bool {
        self.blocked().await || !self.checkpoint.lock().await.retries.is_empty()
    }

    pub(super) async fn begin(
        &self,
        state: Value,
        key: String,
        idempotent: bool,
    ) -> Result<(), DeliveryResult> {
        if state.is_null() {
            self.failed.store(true, Ordering::SeqCst);
        }
        let mut checkpoint = self.checkpoint.lock().await;
        if self.failed.load(Ordering::SeqCst) || (!idempotent && !checkpoint.uncertain.is_empty()) {
            return Err(DeliveryResult::Unknown {
                confirmed: Vec::new(),
            });
        }
        if checkpoint.refused.contains(&key) {
            return Err(DeliveryResult::PermanentError(
                "provider refused this operation".into(),
            ));
        }
        if let Some(deadline) = checkpoint
            .retries
            .get(&key)
            .filter(|deadline| **deadline > now_millis())
        {
            return Err(DeliveryResult::RateLimited {
                retry_after: std::time::Duration::from_millis(
                    deadline.saturating_sub(now_millis()),
                ),
            });
        }
        self.dirty.store(true, Ordering::SeqCst);
        checkpoint.state = state;
        checkpoint.in_flight = Some(DeliveryAttempt {
            key,
            confirmed: Vec::new(),
            idempotent,
            outcome: None,
        });
        self.persist(&checkpoint)
            .await
            .map_err(|_| DeliveryResult::Unknown {
                confirmed: Vec::new(),
            })
    }

    pub(super) async fn outcome(&self, result: &DeliveryResult) {
        let mut checkpoint = self.checkpoint.lock().await;
        let Some(mut attempt) = checkpoint.in_flight.take() else {
            return;
        };
        for reference in result.confirmed() {
            if !attempt.confirmed.contains(reference) {
                attempt.confirmed.push(reference.clone());
            }
        }
        self.dirty.store(true, Ordering::SeqCst);
        attempt.outcome = Some(result.clone());
        if (result.is_uncertain() || matches!(result, DeliveryResult::Partial { .. }))
            && !attempt.idempotent
        {
            checkpoint.uncertain.push(attempt);
            let _ = self.persist(&checkpoint).await;
        } else {
            let advisory = matches!(attempt.key.as_str(), "status" | "title")
                || attempt.key.starts_with("reaction:");
            match result {
                _ if advisory => {
                    checkpoint.retries.remove(&attempt.key);
                }
                DeliveryResult::PermanentError(_) => {
                    checkpoint.retries.remove(&attempt.key);
                    checkpoint.refused.insert(attempt.key.clone());
                }
                DeliveryResult::TransientError(_) | DeliveryResult::RateLimited { .. } => {
                    let wait = result
                        .retry_after()
                        .unwrap_or(std::time::Duration::from_secs(1));
                    checkpoint.retries.insert(
                        attempt.key.clone(),
                        now_millis()
                            .saturating_add(u64::try_from(wait.as_millis()).unwrap_or(u64::MAX)),
                    );
                }
                DeliveryResult::Unknown { .. } if attempt.idempotent => {
                    checkpoint
                        .retries
                        .insert(attempt.key.clone(), now_millis().saturating_add(1000));
                }
                _ => {
                    checkpoint.retries.remove(&attempt.key);
                }
            }
            // Until the updated consumer state is committed, retain the intent on disk.
            // A crash here must remain uncertain, even after an apparently successful call.
        }
    }

    pub(super) async fn save(&self, state: Value) -> Result<(), ChannelError> {
        if state.is_null() {
            self.failed.store(true, Ordering::SeqCst);
            return Err(ChannelError::Session(
                "delivery state could not be encoded".into(),
            ));
        }
        let mut checkpoint = self.checkpoint.lock().await;
        checkpoint.state = state;
        self.persist(&checkpoint).await?;
        self.dirty.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl DeliveryReceiptSink for DeliveryJournal {
    async fn confirmed(&self, reference: &str) -> Result<(), ChannelError> {
        // These are identifiers, not credential-bearing URLs or arbitrary responses.
        if reference.is_empty() || reference.len() > 4096 || reference.contains("://") {
            return Err(ChannelError::Session(
                "invalid delivery receipt reference".into(),
            ));
        }
        let mut checkpoint = self.checkpoint.lock().await;
        let attempt = checkpoint.in_flight.as_mut().ok_or_else(|| {
            ChannelError::Session("delivery has no committed output intent".into())
        })?;
        if !attempt
            .confirmed
            .iter()
            .any(|existing| existing == reference)
        {
            if attempt.confirmed.len() >= MAX_PARTS {
                return Err(ChannelError::Session(
                    "delivery receipt capacity exceeded".into(),
                ));
            }
            attempt.confirmed.push(reference.into());
        }
        self.persist(&checkpoint).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_runtime::{DeliveryTarget, MemoryChannelStore, PendingDelivery};
    use serde_json::json;
    use std::time::Duration;

    async fn store(lease: Duration) -> Arc<MemoryChannelStore> {
        let store = Arc::new(MemoryChannelStore::new());
        store
            .save_pending(
                &PendingDelivery {
                    channel: "install-a".into(),
                    session_id: "session-a".into(),
                    input_message_id: "input-a".into(),
                    target: DeliveryTarget::new("C1", "1.0"),
                    after_sequence: 0,
                },
                "owner-a",
                lease,
            )
            .await
            .unwrap();
        store
    }

    fn journal(store: Arc<MemoryChannelStore>, checkpoint: DeliveryCheckpoint) -> DeliveryJournal {
        DeliveryJournal::new(
            store,
            "session-a".into(),
            "input-a".into(),
            "owner-a".into(),
            checkpoint,
        )
    }

    #[tokio::test]
    async fn interrupted_send_retains_confirmed_parts_and_never_reopens_an_intent() {
        let store = store(Duration::from_secs(60)).await;
        let first = journal(store.clone(), DeliveryCheckpoint::default());
        first
            .begin(json!({"message":"output-a"}), "post:output-a".into(), false)
            .await
            .unwrap();
        first.confirmed("C1:2.0").await.unwrap();
        let saved = store
            .delivery_checkpoint("session-a", "input-a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.in_flight.as_ref().unwrap().confirmed, ["C1:2.0"]);
        let recovered = journal(store.clone(), saved);
        assert!(recovered.blocked().await);
        assert!(matches!(
            recovered
                .begin(json!({}), "post:output-a".into(), false)
                .await,
            Err(DeliveryResult::Unknown { .. })
        ));
        assert!(
            recovered
                .confirmed("https://hooks.slack.com/private-secret")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn settlement_commits_consumer_state_and_rate_advice_survives_restart() {
        let store = store(Duration::from_secs(60)).await;
        let first = journal(store.clone(), DeliveryCheckpoint::default());
        first
            .begin(json!({"done":false}), "post:output-a".into(), false)
            .await
            .unwrap();
        first
            .outcome(&DeliveryResult::RateLimited {
                retry_after: Duration::from_secs(30),
            })
            .await;
        first.save(json!({"done":false})).await.unwrap();
        let saved = store
            .delivery_checkpoint("session-a", "input-a")
            .await
            .unwrap()
            .unwrap();
        assert!(saved.in_flight.is_none());
        let recovered = journal(store.clone(), saved);
        assert!(matches!(
            recovered
                .begin(json!({}), "post:output-a".into(), false)
                .await,
            Err(DeliveryResult::RateLimited { .. })
        ));
        // A distinct operation cannot erase the first operation's retry obligation.
        recovered
            .begin(json!({}), "title".into(), true)
            .await
            .unwrap();
        recovered.outcome(&DeliveryResult::Ok).await;
        recovered.save(json!({"title":"set"})).await.unwrap();
        assert!(recovered.pending().await);
    }

    #[tokio::test]
    async fn partial_refusal_requires_reconciliation_and_cleanup_retains_it() {
        let store = store(Duration::from_secs(60)).await;
        let first = journal(store.clone(), DeliveryCheckpoint::default());
        first.begin(json!({}), "post".into(), false).await.unwrap();
        first
            .outcome(&DeliveryResult::Partial {
                confirmed: vec!["C1:2.0".into()],
                cause: Box::new(DeliveryResult::RateLimited {
                    retry_after: Duration::from_secs(1),
                }),
            })
            .await;
        assert!(first.blocked().await);
        first
            .begin(json!({}), "stop:known-handle".into(), true)
            .await
            .unwrap();
        first.outcome(&DeliveryResult::Ok).await;
        first.save(json!({"closed":true})).await.unwrap();
        let saved = store
            .delivery_checkpoint("session-a", "input-a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.uncertain[0].confirmed, ["C1:2.0"]);
        assert!(journal(store, saved).blocked().await);
    }

    #[tokio::test]
    async fn expired_holder_cannot_write_receipts_or_clear_a_successors_delivery() {
        let store = store(Duration::ZERO).await;
        let first = journal(store.clone(), DeliveryCheckpoint::default());
        assert_eq!(
            store
                .claim_pending("owner-b", Duration::from_secs(60))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(first.begin(json!({}), "post".into(), false).await.is_err());
        assert!(
            store
                .complete_delivery("session-a", "input-a", "owner-a")
                .await
                .is_err()
        );
        assert!(first.blocked().await);
        store
            .save_delivery_checkpoint(
                "session-a",
                "input-a",
                "owner-b",
                &DeliveryCheckpoint::default(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn duplicate_pending_insert_cannot_take_the_current_lease() {
        let store = store(Duration::from_secs(60)).await;
        let pending = store.pending_for_test().await.unwrap().remove(0);
        store
            .save_pending(&pending, "owner-b", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(
            store
                .complete_delivery("session-a", "input-a", "owner-b")
                .await
                .is_err()
        );
        store
            .complete_delivery("session-a", "input-a", "owner-a")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn permanent_refusal_does_not_dispatch_again_after_restart() {
        let store = store(Duration::from_secs(60)).await;
        let first = journal(store.clone(), DeliveryCheckpoint::default());
        first.begin(json!({}), "post".into(), false).await.unwrap();
        first
            .outcome(&DeliveryResult::PermanentError(
                "invalid destination".into(),
            ))
            .await;
        first.save(json!({})).await.unwrap();
        let saved = store
            .delivery_checkpoint("session-a", "input-a")
            .await
            .unwrap()
            .unwrap();
        let recovered = journal(store, saved);
        assert!(matches!(
            recovered.begin(json!({}), "post".into(), false).await,
            Err(DeliveryResult::PermanentError(_))
        ));
        assert!(!recovered.pending().await);
        assert!(
            recovered
                .begin(json!({}), "notice".into(), false)
                .await
                .is_ok()
        );
    }
}
