//! ChatGPT-plan and legacy Codex providers. Hosts own browser and credential storage.
pub mod auth;
mod driver;
pub mod login;
pub mod oauth;
pub use driver::{ChatGptChatDriver, REJECTED_FIELDS, descriptor, provider, register_driver};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexAuth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix epoch milliseconds.
    pub expires_at: Option<i64>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub client_id: Option<String>,
    pub open_source: Option<OpenSourceGrant>,
}
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpenSourceGrant {
    pub id_token: Option<String>,
    pub scopes: Vec<String>,
    pub subject: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptRegistration {
    pub client_id: String,
    pub subject: String,
    pub email: Option<String>,
}
impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAuth")
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}
impl std::fmt::Debug for OpenSourceGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSourceGrant")
            .field("scopes", &self.scopes)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}
