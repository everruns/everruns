// Session storage that refreshes the turn's MCP tools after a reveal.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D6).
//
// Why: a deferred MCP server is revealed by writing a session record (tool
// search, or a call to the server's placeholder). The control plane lists the
// server's tools when it next builds the turn context, but this worker keeps
// the turn context for the whole turn (`turn_reads`), so the model would only
// get the tools on its next message. Dropping the session's kept reads when a
// reveal is written makes the next step of the same turn load them.
//
// Decision: only reveal writes refresh. Other session records (ARD
// attachments included) keep their next-turn semantics.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::mcp_deferred::DEFERRED_MCP_REVEAL_KV_PREFIX;
use everruns_contracts::runtime::{BoundSecret, BoundSecretInfo};
use everruns_contracts::typed_id::SessionId;

use crate::core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use crate::phase_reads::PhaseReads;

pub(crate) struct RevealAwareStorage {
    pub(crate) inner: Arc<dyn SessionStorageStore>,
    pub(crate) reads: PhaseReads,
    pub(crate) org_id: i64,
}

#[async_trait]
impl SessionStorageStore for RevealAwareStorage {
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        self.inner.set_value(session_id, key, value).await?;
        if key.starts_with(DEFERRED_MCP_REVEAL_KV_PREFIX) {
            self.reads
                .invalidate_session(self.org_id, session_id.uuid());
        }
        Ok(())
    }

    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        self.inner.get_value(session_id, key).await
    }

    async fn take_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        self.inner.take_value(session_id, key).await
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        self.inner.delete_value(session_id, key).await
    }

    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        self.inner.list_keys(session_id).await
    }

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.inner.set_secret(session_id, name, value).await
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        self.inner.get_secret(session_id, name).await
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        self.inner.delete_secret(session_id, name).await
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        self.inner.list_secrets(session_id).await
    }

    // Decision: forward explicitly. These methods default to "none", so a
    // wrapper that skips them silently hides every bound secret from the
    // shell (the key then never reaches the bound API).
    async fn list_bound_secrets(&self, session_id: SessionId) -> Result<Vec<BoundSecretInfo>> {
        self.inner.list_bound_secrets(session_id).await
    }

    async fn resolve_bound_secrets(&self, session_id: SessionId) -> Result<Vec<BoundSecret>> {
        self.inner.resolve_bound_secrets(session_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn_reads::{TurnSlot, TurnValues};
    use std::collections::HashMap;

    fn sessions(
        values: &mut TurnValues,
    ) -> &mut HashMap<(i64, uuid::Uuid), Option<crate::core::ExecutionSession>> {
        &mut values.sessions
    }

    #[tokio::test]
    async fn a_reveal_drops_the_turns_kept_reads_and_other_writes_do_not() {
        let session = SessionId::new();
        let slot = TurnSlot::default();
        let keep = || slot.put(sessions, (7, session.uuid()), None);
        let storage = RevealAwareStorage {
            inner: Arc::new(crate::core::host::InMemorySessionStorageStore::new()),
            reads: PhaseReads::new().with_turn(slot.clone()),
            org_id: 7,
        };

        keep();
        storage.set_value(session, "notes", "x").await.unwrap();
        assert!(slot.get(sessions, &(7, session.uuid())).is_some());

        storage
            .set_value(session, "mcp_reveal:linear", "1")
            .await
            .unwrap();
        assert!(slot.get(sessions, &(7, session.uuid())).is_none());
        assert_eq!(
            storage
                .get_value(session, "mcp_reveal:linear")
                .await
                .unwrap()
                .as_deref(),
            Some("1")
        );
    }

    /// The shell resolves bound secrets through this wrapper; dropping them
    /// sent `Authorization: Bearer ` with no key to the bound API.
    #[tokio::test]
    async fn bound_secrets_pass_through_to_the_inner_store() {
        use everruns_contracts::runtime::{SecretAuth, SecretBinding, SecretScope};

        struct Bound(BoundSecret);

        #[async_trait]
        impl SessionStorageStore for Bound {
            async fn set_value(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            async fn get_value(&self, _: SessionId, _: &str) -> Result<Option<String>> {
                Ok(None)
            }
            async fn delete_value(&self, _: SessionId, _: &str) -> Result<bool> {
                Ok(false)
            }
            async fn list_keys(&self, _: SessionId) -> Result<Vec<KeyInfo>> {
                Ok(Vec::new())
            }
            async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            async fn get_secret(&self, _: SessionId, _: &str) -> Result<Option<String>> {
                Ok(None)
            }
            async fn delete_secret(&self, _: SessionId, _: &str) -> Result<bool> {
                Ok(false)
            }
            async fn list_secrets(&self, _: SessionId) -> Result<Vec<SecretInfo>> {
                Ok(Vec::new())
            }
            async fn list_bound_secrets(&self, _: SessionId) -> Result<Vec<BoundSecretInfo>> {
                Ok(vec![self.0.info.clone()])
            }
            async fn resolve_bound_secrets(&self, _: SessionId) -> Result<Vec<BoundSecret>> {
                Ok(vec![self.0.clone()])
            }
        }

        let secret = BoundSecret {
            info: BoundSecretInfo {
                name: "JEV_API_KEY".to_string(),
                binding: SecretBinding {
                    url: "https://api.typesafe.ai".to_string(),
                    auth: SecretAuth::Bearer,
                },
                scope: SecretScope::Agent,
            },
            value: "sk-live-value".to_string(),
        };
        let storage = RevealAwareStorage {
            inner: Arc::new(Bound(secret)),
            reads: PhaseReads::new(),
            org_id: 7,
        };
        let session = SessionId::new();

        let listed = storage.list_bound_secrets(session).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "JEV_API_KEY");
        let resolved = storage.resolve_bound_secrets(session).await.unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].value, "sk-live-value");
    }
}
