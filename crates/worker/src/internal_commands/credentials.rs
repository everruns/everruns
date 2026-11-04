//! One secret-safe command path for in-process and remote credential services.
use super::session_storage::CommandSessionStorageStore;
use super::{InternalCommandTransport, call_secret};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::credential_services::*;
use everruns_contracts::runtime::resource_access::CredentialUsage;
use serde_json::{Value, json};
use uuid::Uuid;

#[async_trait]
impl<T: InternalCommandTransport + 'static> CredentialService for CommandSessionStorageStore<T> {
    async fn list(
        &self,
        invocation: CredentialInvocation,
        usage: CredentialUsage,
    ) -> Result<Vec<CredentialInfo>> {
        self.credentials_call(invocation, "list", json!({"usage":usage}))
            .await
    }
    async fn resolve(
        &self,
        invocation: CredentialInvocation,
        usage: CredentialUsage,
        names: Vec<String>,
    ) -> Result<Vec<ResolvedCredential>> {
        self.credentials_call(invocation, "resolve", json!({"usage":usage,"names":names}))
            .await
    }
    async fn prepare_capture(
        &self,
        invocation: CredentialInvocation,
        plan: CapturePlan,
    ) -> Result<CapturePreparation> {
        self.credentials_call(
            invocation,
            "prepare",
            serde_json::to_value(plan).unwrap_or_default(),
        )
        .await
    }
    async fn commit_capture(
        &self,
        invocation: CredentialInvocation,
        ticket: Uuid,
        values: Vec<CapturedCredential>,
        result: Value,
    ) -> Result<()> {
        self.credentials_call(
            invocation,
            "commit",
            json!({"ticket":ticket,"values":values,"result":result}),
        )
        .await
    }
}

impl<T: InternalCommandTransport> CommandSessionStorageStore<T> {
    async fn credentials_call<R: serde::de::DeserializeOwned>(
        &self,
        invocation: CredentialInvocation,
        operation: &str,
        params: Value,
    ) -> Result<R> {
        call_secret(self.transport.as_ref(), "Credential service", "worker_credentials",
            json!({"session_id":invocation.session_id.to_string(),"input_message_id":invocation.input_message_id,"operation":operation,"params":params})).await
    }
}
