use crate::dreaming::{DreamStateRow, DreamingStore};
use crate::{error::Result, typed_id::SessionId};
use async_trait::async_trait;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;
#[derive(Clone, Default)]
pub struct InMemoryDreamingStore(Arc<Mutex<BTreeMap<(i64, String), DreamStateRow>>>);
#[async_trait]
impl DreamingStore for InMemoryDreamingStore {
    async fn get(&self, org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        Ok(self.0.lock().await.get(&(org, key.into())).cloned())
    }
    async fn compare_exchange(
        &self,
        org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool> {
        let mut rows = self.0.lock().await;
        let k = (org, key.into());
        if rows.get(&k).map(|r| r.revision) != expected {
            return Ok(false);
        }
        rows.insert(
            k,
            DreamStateRow {
                revision: expected.unwrap_or(0) + 1,
                value: value.into(),
                session_id,
                due_at,
            },
        );
        Ok(true)
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        Ok(self
            .0
            .lock()
            .await
            .iter()
            .filter_map(|((org, _), r)| {
                r.session_id
                    .filter(|_| r.due_at.is_some_and(|d| d <= now))
                    .map(|id| (*org, id))
            })
            .take(limit.min(100) as usize)
            .collect())
    }
}

/// Durable hosts must opt into a coherent store instead of inheriting volatile state.
pub struct UnsupportedDreamingStore;
#[async_trait]
impl DreamingStore for UnsupportedDreamingStore {
    async fn get(&self, _: i64, _: &str) -> Result<Option<DreamStateRow>> {
        Err(crate::error::AgentLoopError::config(
            "host must supply durable dreaming storage",
        ))
    }
    async fn compare_exchange(
        &self,
        _: i64,
        _: &str,
        _: Option<i64>,
        _: &str,
        _: Option<SessionId>,
        _: Option<i64>,
    ) -> Result<bool> {
        Err(crate::error::AgentLoopError::config(
            "host must supply durable dreaming storage",
        ))
    }
    async fn due(&self, _: i64, _: u32) -> Result<Vec<(i64, SessionId)>> {
        Ok(Vec::new())
    }
}
