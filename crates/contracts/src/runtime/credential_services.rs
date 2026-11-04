//! Write-only credential use and capture across every hosting transport.

use super::resource_access::{CredentialUsage, ResourceScope};
use super::session_services::SecretBinding;
use super::typed_id::SessionId;
use crate::error::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Set by the Engine when constructing the per-call context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CredentialInvocation {
    pub session_id: SessionId,
    pub input_message_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialInfo {
    pub id: Uuid,
    pub revision: i64,
    pub name: String,
    pub binding: SecretBinding,
    pub scope: ResourceScope,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResolvedCredential {
    pub info: CredentialInfo,
    pub value: String,
}

impl std::fmt::Debug for ResolvedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedCredential")
            .field("info", &self.info)
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSpec {
    pub name: String,
    pub from: String,
    pub scope: ResourceScope,
    pub binding: SecretBinding,
    #[serde(default)]
    pub expires_in_from: Option<String>,
    #[serde(default)]
    pub expires_at_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturePlan {
    pub usage: CredentialUsage,
    pub source_url: String,
    /// Exact post-hook arguments, containing references only.
    pub arguments: Value,
    pub captures: Vec<CaptureSpec>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CapturedCredential {
    pub name: String,
    pub value: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for CapturedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedCredential")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CapturePreparation {
    Ready {
        ticket: Uuid,
    },
    ConsentRequired {
        ticket: Uuid,
        expires_at: DateTime<Utc>,
        summary: Value,
    },
    Completed {
        result: Value,
    },
}

#[async_trait]
pub trait CredentialService: Send + Sync {
    async fn list(
        &self,
        invocation: CredentialInvocation,
        usage: CredentialUsage,
    ) -> Result<Vec<CredentialInfo>>;
    async fn resolve(
        &self,
        invocation: CredentialInvocation,
        usage: CredentialUsage,
        names: Vec<String>,
    ) -> Result<Vec<ResolvedCredential>>;
    async fn prepare_capture(
        &self,
        invocation: CredentialInvocation,
        plan: CapturePlan,
    ) -> Result<CapturePreparation>;
    async fn commit_capture(
        &self,
        invocation: CredentialInvocation,
        ticket: Uuid,
        values: Vec<CapturedCredential>,
        result: Value,
    ) -> Result<()>;
}
