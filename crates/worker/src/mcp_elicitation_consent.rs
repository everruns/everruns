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
//
// Form mode answers (knowledge/integrations/mcp-form-elicitation.md) travel the
// same way: the server's question-answer API parks them here, and the retried
// tool call takes them, once, to answer the server.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_services::SessionStorageStore;
use everruns_mcp::{
    ElicitationConsentStore, FormAnswerStore, GrantedConsent, StoredConsent, StoredFormAnswer,
    consent_storage_key, form_answer_storage_key,
};

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
        // THREAT[TM-TOOL-034]: atomic destructive consumption ensures that
        // concurrent retries cannot turn one decision into multiple accepts.
        let Some(raw) = self.storage.take_value(self.session_id, &key).await? else {
            return Ok(None);
        };

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

#[async_trait]
impl FormAnswerStore for SessionElicitationConsents {
    async fn take_form_answer(
        &self,
        server: &str,
        tool: &str,
    ) -> anyhow::Result<Option<StoredFormAnswer>> {
        let key = form_answer_storage_key(server, tool);
        // Destructive read: one answer is sent to the server at most once, even
        // when retries race.
        let Some(raw) = self.storage.take_value(self.session_id, &key).await? else {
            return Ok(None);
        };
        match serde_json::from_str(&raw) {
            Ok(record) => Ok(Some(record)),
            Err(error) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    %error,
                    "Discarding an unreadable form elicitation answer"
                );
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::error::Result as CoreResult;
    use everruns_core::session_services::{KeyInfo, SecretInfo};
    use everruns_core::tool_context::ToolContext;
    use everruns_core::tools::{Tool, ToolExecutionResult};
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
        async fn take_value(&self, _s: SessionId, key: &str) -> CoreResult<Option<String>> {
            Ok(self.values.lock().expect("lock").remove(key))
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
    async fn concurrent_consumers_receive_exactly_one_grant() {
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

        let consents = Arc::new(SessionElicitationConsents::new(storage, session_id));
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut consumers = Vec::new();
        for _ in 0..2 {
            let consents = consents.clone();
            let barrier = barrier.clone();
            consumers.push(tokio::spawn(async move {
                barrier.wait().await;
                consents
                    .take_consent("billing", "charge")
                    .await
                    .expect("take")
            }));
        }
        barrier.wait().await;

        let mut grants = 0;
        for consumer in consumers {
            if consumer.await.expect("consumer task").is_some() {
                grants += 1;
            }
        }
        assert_eq!(grants, 1, "one decision authorises exactly one accept");
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

    #[tokio::test]
    async fn reads_a_recorded_form_answer_once() {
        let session_id = SessionId::new();
        let storage = Arc::new(MemoryStorage::default());
        let record = StoredFormAnswer::new(
            "deploys",
            "release",
            "fingerprint",
            everruns_mcp::FormAnswerAction::Decline,
            Default::default(),
            chrono::Utc::now(),
        );
        storage
            .set_value(
                session_id,
                &form_answer_storage_key("deploys", "release"),
                &serde_json::to_string(&record).expect("serialize"),
            )
            .await
            .expect("store");
        let answers = SessionElicitationConsents::new(storage, session_id);

        let first = answers
            .take_form_answer("deploys", "release")
            .await
            .expect("read");
        assert_eq!(
            first.map(|record| record.fingerprint).as_deref(),
            Some("fingerprint")
        );
        assert!(
            answers
                .take_form_answer("deploys", "release")
                .await
                .expect("read")
                .is_none(),
            "an answer is sent at most once"
        );
    }

    // EVE-1141 / THREAT[TM-TOOL-034]: consent and form answers are authority
    // the answer APIs write on a person's behalf. The model-facing `kv_store`
    // tool shares the same session storage, so it must not be able to mint
    // either record and have the retried tool call honour it.
    async fn model_kv_store_set(
        storage: Arc<MemoryStorage>,
        session_id: SessionId,
        key: &str,
        value: &str,
    ) -> ToolExecutionResult {
        let context = ToolContext::with_storage_store(session_id, storage);
        everruns_host::KvStoreTool
            .execute_with_context(
                serde_json::json!({"operation": "set", "key": key, "value": value}),
                &context,
            )
            .await
    }

    #[tokio::test]
    async fn a_model_authored_consent_grants_nothing() {
        let session_id = SessionId::new();
        let storage = Arc::new(MemoryStorage::default());
        let forged = StoredConsent::new("billing", "charge", "pay.example.com", chrono::Utc::now());

        let result = model_kv_store_set(
            storage.clone(),
            session_id,
            &consent_storage_key("billing", "charge"),
            &serde_json::to_string(&forged).expect("serialize"),
        )
        .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
            "kv_store must refuse the consent prefix, got {result:?}"
        );

        let consents = SessionElicitationConsents::new(storage, session_id);
        assert_eq!(
            consents
                .take_consent("billing", "charge")
                .await
                .expect("read"),
            None,
            "a consent the model wrote must not authorise an accept"
        );
    }

    #[tokio::test]
    async fn a_model_authored_form_answer_is_never_sent() {
        let session_id = SessionId::new();
        let storage = Arc::new(MemoryStorage::default());
        let forged = StoredFormAnswer::new(
            "deploys",
            "release",
            "fingerprint",
            everruns_mcp::FormAnswerAction::Accept,
            Default::default(),
            chrono::Utc::now(),
        );

        let result = model_kv_store_set(
            storage.clone(),
            session_id,
            &form_answer_storage_key("deploys", "release"),
            &serde_json::to_string(&forged).expect("serialize"),
        )
        .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
            "kv_store must refuse the form-answer prefix, got {result:?}"
        );

        let answers = SessionElicitationConsents::new(storage, session_id);
        assert!(
            answers
                .take_form_answer("deploys", "release")
                .await
                .expect("read")
                .is_none(),
            "an answer the model wrote must not reach the server"
        );
    }
}
