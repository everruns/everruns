//! `SessionStorageStore` whose key/value half runs the
//! `worker_*_session_storage_value(s)` commands.
//!
//! Only the values moved. Secrets stay on their dedicated
//! `SessionStorage*Secret` RPCs (or, in-process, the database store) until
//! they move with connections and credentials, so the store takes the secret
//! half as a separate [`SessionSecretStorage`].

use super::{InternalCommandTransport, call};
use crate::core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;
use std::sync::Arc;

/// The secret half of session storage, reached apart from the values.
#[async_trait]
pub trait SessionSecretStorage: Send + Sync {
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()>;
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>>;
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool>;
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>>;
}

/// A full store (the in-process worker's database store) lends its secrets.
#[async_trait]
impl<S: SessionStorageStore + ?Sized> SessionSecretStorage for Arc<S> {
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        (**self).set_secret(session_id, name, value).await
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        (**self).get_secret(session_id, name).await
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        (**self).delete_secret(session_id, name).await
    }
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        (**self).list_secrets(session_id).await
    }
}

/// The session storage tools use, on either transport.
pub struct CommandSessionStorageStore<T, S> {
    transport: T,
    secrets: S,
}

impl<T: InternalCommandTransport, S: SessionSecretStorage> CommandSessionStorageStore<T, S> {
    pub fn new(transport: T, secrets: S) -> Self {
        Self { transport, secrets }
    }
}

/// The command's key listing, as it crosses the wire.
#[derive(serde::Deserialize)]
struct StorageKey {
    key: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn entry(session_id: SessionId, key: &str) -> serde_json::Value {
    json!({ "session_id": session_id.to_string(), "key": key })
}

#[async_trait]
impl<T: InternalCommandTransport, S: SessionSecretStorage> SessionStorageStore
    for CommandSessionStorageStore<T, S>
{
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        call(
            &self.transport,
            "Set storage value",
            "worker_set_session_storage_value",
            json!({ "session_id": session_id.to_string(), "key": key, "value": value }),
        )
        .await
    }

    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        call(
            &self.transport,
            "Get storage value",
            "worker_get_session_storage_value",
            entry(session_id, key),
        )
        .await
    }

    /// Atomic on the server, which matters: a store shared between callers
    /// must not take the trait's get-then-delete default.
    async fn take_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        call(
            &self.transport,
            "Take storage value",
            "worker_take_session_storage_value",
            entry(session_id, key),
        )
        .await
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        call(
            &self.transport,
            "Delete storage value",
            "worker_delete_session_storage_value",
            entry(session_id, key),
        )
        .await
    }

    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        let keys: Vec<StorageKey> = call(
            &self.transport,
            "List storage keys",
            "worker_list_session_storage_keys",
            json!({ "session_id": session_id.to_string() }),
        )
        .await?;
        Ok(keys
            .into_iter()
            .map(|key| KeyInfo {
                key: key.key,
                created_at: key.created_at,
                updated_at: key.updated_at,
            })
            .collect())
    }

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.secrets.set_secret(session_id, name, value).await
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        self.secrets.get_secret(session_id, name).await
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        self.secrets.delete_secret(session_id, name).await
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        self.secrets.list_secrets(session_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::host::InMemorySessionStorageStore;
    use everruns_internal_protocol::proto;
    use serde_json::Value;
    use std::sync::Mutex;

    /// Records each call and answers with a canned result.
    struct Recorded {
        calls: Mutex<Vec<(String, Value)>>,
        answer: std::result::Result<Value, proto::CommandError>,
    }

    #[async_trait]
    impl InternalCommandTransport for &Recorded {
        async fn execute_internal_command(
            &self,
            name: &str,
            params: Value,
        ) -> Result<std::result::Result<Value, proto::CommandError>> {
            self.calls.lock().unwrap().push((name.to_string(), params));
            Ok(self.answer.clone())
        }
    }

    fn recorded(answer: std::result::Result<Value, proto::CommandError>) -> Recorded {
        Recorded {
            calls: Mutex::new(Vec::new()),
            answer,
        }
    }

    fn secrets() -> Arc<InMemorySessionStorageStore> {
        Arc::new(InMemorySessionStorageStore::new())
    }

    #[tokio::test]
    async fn values_travel_as_commands_and_decode_back() {
        let session = SessionId::new();
        let answered = recorded(Ok(json!([{
            "key": "a",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
        }])));
        let store = CommandSessionStorageStore::new(&answered, secrets());
        let keys = store.list_keys(session).await.unwrap();
        assert_eq!(keys[0].key, "a");
        assert_eq!(keys[0].updated_at.to_rfc3339(), "2026-01-02T00:00:00+00:00");

        let set = recorded(Ok(Value::Null));
        CommandSessionStorageStore::new(&set, secrets())
            .set_value(session, "k", "v")
            .await
            .unwrap();
        let calls = set.calls.lock().unwrap();
        let (name, params) = &calls[0];
        assert_eq!(name, "worker_set_session_storage_value");
        assert_eq!(params["session_id"], session.to_string());
        assert_eq!(params["key"], "k");
        assert_eq!(params["value"], "v");
    }

    #[tokio::test]
    async fn take_is_one_command_and_absent_values_decode() {
        let session = SessionId::new();
        let absent = recorded(Ok(Value::Null));
        let store = CommandSessionStorageStore::new(&absent, secrets());
        assert!(store.take_value(session, "gone").await.unwrap().is_none());
        assert!(store.get_value(session, "gone").await.unwrap().is_none());
        let calls = absent.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "take is not a get then a delete");
        assert_eq!(calls[0].0, "worker_take_session_storage_value");
        assert_eq!(calls[0].1["key"], "gone");
    }

    #[tokio::test]
    async fn secrets_stay_off_the_command_transport() {
        let session = SessionId::new();
        let unused = recorded(Ok(Value::Null));
        let store = CommandSessionStorageStore::new(&unused, secrets());
        store.set_secret(session, "TOKEN", "s3cret").await.unwrap();
        assert_eq!(
            store.get_secret(session, "TOKEN").await.unwrap().as_deref(),
            Some("s3cret")
        );
        assert!(store.delete_secret(session, "TOKEN").await.unwrap());
        assert!(store.list_secrets(session).await.unwrap().is_empty());
        assert!(unused.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failures_name_the_operation() {
        let missing = recorded(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Session".to_string(),
        }));
        let error = CommandSessionStorageStore::new(&missing, secrets())
            .delete_value(SessionId::new(), "a")
            .await
            .expect_err("not found");
        assert!(
            error.to_string().contains("Delete storage value: Session"),
            "{error}"
        );
    }
}
