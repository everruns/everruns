//! `SessionStorageStore` over the `worker_*_session_storage_value(s)` and
//! `worker_*_session_secret(s)` commands.
//!
//! Decision: values and secrets both travel as internal commands, so the store
//! needs nothing but a transport. Secrets used to stay on dedicated RPCs (and,
//! in-process, on the database store) while connections and credentials were
//! still RPCs; they moved together. A secret's value crosses only in the
//! command's params (set) or answer (get), never in a failure message: see
//! [`super::call_secret`].

use super::{InternalCommandTransport, call, call_secret};
use crate::core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;

/// The session storage tools use, on either transport.
pub struct CommandSessionStorageStore<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandSessionStorageStore<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

/// A key or secret listing, as it crosses the wire.
#[derive(serde::Deserialize)]
struct Listed {
    #[serde(alias = "name")]
    key: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn entry(session_id: SessionId, key: &str) -> serde_json::Value {
    json!({ "session_id": session_id.to_string(), "key": key })
}

fn secret(session_id: SessionId, name: &str) -> serde_json::Value {
    json!({ "session_id": session_id.to_string(), "name": name })
}

#[async_trait]
impl<T: InternalCommandTransport> SessionStorageStore for CommandSessionStorageStore<T> {
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
        let keys: Vec<Listed> = call(
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
        call_secret(
            &self.transport,
            "Set secret",
            "worker_set_session_secret",
            json!({ "session_id": session_id.to_string(), "name": name, "value": value }),
        )
        .await
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        call_secret(
            &self.transport,
            "Get secret",
            "worker_get_session_secret",
            secret(session_id, name),
        )
        .await
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        call(
            &self.transport,
            "Delete secret",
            "worker_delete_session_secret",
            secret(session_id, name),
        )
        .await
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let secrets: Vec<Listed> = call(
            &self.transport,
            "List secrets",
            "worker_list_session_secrets",
            json!({ "session_id": session_id.to_string() }),
        )
        .await?;
        Ok(secrets
            .into_iter()
            .map(|secret| SecretInfo {
                name: secret.key,
                created_at: secret.created_at,
                updated_at: secret.updated_at,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn values_travel_as_commands_and_decode_back() {
        let session = SessionId::new();
        let answered = recorded(Ok(json!([{
            "key": "a",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
        }])));
        let store = CommandSessionStorageStore::new(&answered);
        let keys = store.list_keys(session).await.unwrap();
        assert_eq!(keys[0].key, "a");
        assert_eq!(keys[0].updated_at.to_rfc3339(), "2026-01-02T00:00:00+00:00");

        let set = recorded(Ok(Value::Null));
        CommandSessionStorageStore::new(&set)
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
        let store = CommandSessionStorageStore::new(&absent);
        assert!(store.take_value(session, "gone").await.unwrap().is_none());
        assert!(store.get_value(session, "gone").await.unwrap().is_none());
        let calls = absent.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "take is not a get then a delete");
        assert_eq!(calls[0].0, "worker_take_session_storage_value");
        assert_eq!(calls[0].1["key"], "gone");
    }

    #[tokio::test]
    async fn secrets_travel_as_commands_and_decode_back() {
        let session = SessionId::new();
        let set = recorded(Ok(Value::Null));
        CommandSessionStorageStore::new(&set)
            .set_secret(session, "TOKEN", "s3cret")
            .await
            .unwrap();
        {
            let calls = set.calls.lock().unwrap();
            assert_eq!(calls[0].0, "worker_set_session_secret");
            assert_eq!(calls[0].1["name"], "TOKEN");
            assert_eq!(calls[0].1["value"], "s3cret");
        }

        let got = recorded(Ok(json!("s3cret")));
        let store = CommandSessionStorageStore::new(&got);
        assert_eq!(
            store.get_secret(session, "TOKEN").await.unwrap().as_deref(),
            Some("s3cret")
        );
        assert_eq!(got.calls.lock().unwrap()[0].0, "worker_get_session_secret");

        let listed = recorded(Ok(json!([{
            "name": "TOKEN",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
        }])));
        let secrets = CommandSessionStorageStore::new(&listed)
            .list_secrets(session)
            .await
            .unwrap();
        assert_eq!(secrets[0].name, "TOKEN");
        assert_eq!(
            listed.calls.lock().unwrap()[0].0,
            "worker_list_session_secrets"
        );

        let deleted = recorded(Ok(json!(true)));
        assert!(
            CommandSessionStorageStore::new(&deleted)
                .delete_secret(session, "TOKEN")
                .await
                .unwrap()
        );
        assert_eq!(
            deleted.calls.lock().unwrap()[0].0,
            "worker_delete_session_secret"
        );
    }

    /// A malformed answer to a secret call names the operation, never the
    /// value serde would quote (`invalid type: string "..."`).
    #[tokio::test]
    async fn a_malformed_secret_answer_does_not_echo_the_value() {
        let wrong = recorded(Ok(json!("s3cret-value")));
        let error = CommandSessionStorageStore::new(&wrong)
            .set_secret(SessionId::new(), "TOKEN", "s3cret-value")
            .await
            .expect_err("wrong shape");
        let text = error.to_string();
        assert!(text.contains("Set secret"), "{text}");
        assert!(!text.contains("s3cret-value"), "{text}");
    }

    #[tokio::test]
    async fn failures_name_the_operation() {
        let missing = recorded(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Session".to_string(),
        }));
        let error = CommandSessionStorageStore::new(&missing)
            .delete_value(SessionId::new(), "a")
            .await
            .expect_err("not found");
        assert!(
            error.to_string().contains("Delete storage value: Session"),
            "{error}"
        );
    }
}
