// Reads the consent a user gave for a URL mode elicitation.
//
// The turn that hits an elicitation cannot wait for a browser, so it ends with
// the URL in front of the user; the consent they give lands in session storage
// (written by the server's elicitation consent API), and the *next* run of the
// same tool reads it here and answers the server `accept`.
//
// Session storage is the right home for it because it is durable and
// session-scoped: the retry may well execute in a different worker process than
// the one that asked, so an in-memory record would silently degrade into asking
// the user again every time.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::session_services::SessionStorageStore;
use everruns_mcp::{ElicitationConsentStore, GrantedConsent, StoredConsent, consent_storage_key};
use everruns_provider::typed_id::SessionId;

/// Session-storage-backed [`ElicitationConsentStore`] for one session.
pub struct SessionElicitationConsents {
    storage: Arc<dyn SessionStorageStore>,
    session_id: SessionId,
}

impl SessionElicitationConsents {
    pub fn new(storage: Arc<dyn SessionStorageStore>, session_id: SessionId) -> Self {
        Self {
            storage,
            session_id,
        }
    }
}

#[async_trait]
impl ElicitationConsentStore for SessionElicitationConsents {
    async fn take_consent(
        &self,
        server: &str,
        tool: &str,
    ) -> anyhow::Result<Option<GrantedConsent>> {
        let key = consent_storage_key(server, tool);
        let Some(raw) = self.storage.get_value(self.session_id, &key).await? else {
            return Ok(None);
        };

        // Delete before honouring it: one consent authorises exactly one
        // `accept`, and deleting first means a crash between the two cannot
        // leave a reusable grant behind.
        if let Err(error) = self.storage.delete_value(self.session_id, &key).await {
            tracing::warn!(
                session_id = %self.session_id,
                %error,
                "Could not consume elicitation consent; refusing to use it twice"
            );
            return Ok(None);
        }

        let record: StoredConsent = match serde_json::from_str(&raw) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    %error,
                    "Discarding an unreadable elicitation consent record"
                );
                return Ok(None);
            }
        };
        Ok(record.grant_for(server, tool, chrono::Utc::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::session_services::{KeyInfo, SecretInfo};
    use everruns_provider::error::Result as CoreResult;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStorage {
        values: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl SessionStorageStore for MemoryStorage {
        async fn set_value(&self, _s: SessionId, key: &str, value: &str) -> CoreResult<()> {
            self.values
                .lock()
                .expect("lock")
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        async fn get_value(&self, _s: SessionId, key: &str) -> CoreResult<Option<String>> {
            Ok(self.values.lock().expect("lock").get(key).cloned())
        }
        async fn delete_value(&self, _s: SessionId, key: &str) -> CoreResult<bool> {
            Ok(self.values.lock().expect("lock").remove(key).is_some())
        }
        async fn list_keys(&self, _s: SessionId) -> CoreResult<Vec<KeyInfo>> {
            Ok(vec![])
        }
        async fn set_secret(&self, _s: SessionId, _n: &str, _v: &str) -> CoreResult<()> {
            unimplemented!("secrets are not part of the consent path")
        }
        async fn get_secret(&self, _s: SessionId, _n: &str) -> CoreResult<Option<String>> {
            unimplemented!("secrets are not part of the consent path")
        }
        async fn delete_secret(&self, _s: SessionId, _n: &str) -> CoreResult<bool> {
            unimplemented!("secrets are not part of the consent path")
        }
        async fn list_secrets(&self, _s: SessionId) -> CoreResult<Vec<SecretInfo>> {
            unimplemented!("secrets are not part of the consent path")
        }
    }

    #[tokio::test]
    async fn reads_a_recorded_consent_once() {
        let session_id = SessionId::new();
        let storage = Arc::new(MemoryStorage::default());
        let record = StoredConsent::new("billing", "charge", "pay.example.com", chrono::Utc::now());
        storage
            .set_value(
                session_id,
                &consent_storage_key("billing", "charge"),
                &serde_json::to_string(&record).expect("serialize"),
            )
            .await
            .expect("stored");

        let consents = SessionElicitationConsents::new(storage.clone(), session_id);

        assert_eq!(
            consents
                .take_consent("billing", "charge")
                .await
                .expect("read"),
            Some(GrantedConsent {
                host: "pay.example.com".to_string()
            })
        );
        assert_eq!(
            consents
                .take_consent("billing", "charge")
                .await
                .expect("read"),
            None,
            "the record is consumed, so a second call asks the user again"
        );
    }

    #[tokio::test]
    async fn a_consent_for_another_tool_grants_nothing() {
        let session_id = SessionId::new();
        let storage = Arc::new(MemoryStorage::default());
        let record = StoredConsent::new("billing", "charge", "pay.example.com", chrono::Utc::now());
        // Written under the key of a different tool: the record names what it
        // is for, so the mismatch is caught even if the key were to collide.
        storage
            .set_value(
                session_id,
                &consent_storage_key("billing", "refund"),
                &serde_json::to_string(&record).expect("serialize"),
            )
            .await
            .expect("stored");

        let consents = SessionElicitationConsents::new(storage, session_id);

        assert_eq!(
            consents
                .take_consent("billing", "refund")
                .await
                .expect("read"),
            None
        );
    }

    #[tokio::test]
    async fn no_record_means_no_consent() {
        let consents =
            SessionElicitationConsents::new(Arc::new(MemoryStorage::default()), SessionId::new());
        assert_eq!(
            consents
                .take_consent("billing", "charge")
                .await
                .expect("read"),
            None
        );
    }
}
