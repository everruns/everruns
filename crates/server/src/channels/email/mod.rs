//! Email intake is persisted before HTTP acknowledgement. Batch leases only
//! authorize triage, never a send or the agent's other channel credentials.
pub mod compose;
pub mod connector;
pub mod driver;
pub mod outbox;
pub mod provider;
pub mod queue;
pub mod routes;
pub mod tools;
pub mod worker;
/// An inbox has one receiving agent. Addresses and provider credentials belong
/// to the registry; channel configuration contains only its stable local ID.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailChannelConfig {
    pub inbox_id: uuid::Uuid,
    /// Trusted scope used by the independent relevance gate, never supplied by mail.
    pub purpose: String,
    /// Autonomous execution is opt-in; output still requires separate review.
    #[serde(default)]
    pub auto_dispatch: bool,
    #[serde(default)]
    pub auto_reply: bool,
    #[serde(default)]
    pub reply_template_id: Option<uuid::Uuid>,
}
#[cfg(test)]
mod tests;
