use crate::*;
use async_trait::async_trait;
use std::{collections::HashMap, sync::Mutex};

#[async_trait]
pub trait State: Send + Sync {
    async fn subscribed(&self, key: &ConversationKey) -> Result<bool>;
    async fn subscribe(&self, key: &ConversationKey, value: bool) -> Result<()>;
    async fn get(&self, key: &ConversationKey) -> Result<Option<serde_json::Value>>;
    async fn set(&self, key: &ConversationKey, value: serde_json::Value) -> Result<()>;
}
#[derive(Default)]
struct Entry {
    subscribed: bool,
    value: Option<serde_json::Value>,
}
pub struct MemoryState {
    entries: Mutex<HashMap<ConversationKey, Entry>>,
    capacity: usize,
}
impl Default for MemoryState {
    fn default() -> Self {
        Self::new(256)
    }
}
impl MemoryState {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            capacity,
        }
    }
}
#[async_trait]
impl State for MemoryState {
    async fn subscribed(&self, key: &ConversationKey) -> Result<bool> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .is_some_and(|v| v.subscribed))
    }
    async fn subscribe(&self, key: &ConversationKey, value: bool) -> Result<()> {
        key.validate()?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if !value {
            if let Some(entry) = entries.get_mut(key) {
                entry.subscribed = false;
                if entry.value.is_none() {
                    entries.remove(key);
                }
            }
            return Ok(());
        }
        if !entries.contains_key(key) && entries.len() >= self.capacity {
            return Err(Error::Capacity("state entries"));
        }
        entries.entry(key.clone()).or_default().subscribed = true;
        Ok(())
    }
    async fn get(&self, key: &ConversationKey) -> Result<Option<serde_json::Value>> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .and_then(|e| e.value.clone()))
    }
    async fn set(&self, key: &ConversationKey, value: serde_json::Value) -> Result<()> {
        key.validate()?;
        if serde_json::to_vec(&value)
            .map_err(|_| Error::InvalidInput("state value"))?
            .len()
            > 64 * 1024
        {
            return Err(Error::Capacity("state value"));
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if !entries.contains_key(key) && entries.len() >= self.capacity {
            return Err(Error::Capacity("state entries"));
        }
        entries.entry(key.clone()).or_default().value = Some(value);
        Ok(())
    }
}
