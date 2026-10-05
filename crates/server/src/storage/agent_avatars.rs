//! Agent avatar storage rows. Rendering lives in `domains::agents::avatar`;
//! these rows hold its output.
//!
//! Decision: variants are small pre-rendered PNGs (tens of KB each), so they
//! live in PostgreSQL `bytea` rather than the object store. Reads are a single
//! primary-key lookup behind immutable cache headers.

use uuid::Uuid;

/// One rendered variant to store with a new avatar.
#[derive(Clone, Debug)]
pub struct AgentAvatarVariantInput {
    pub variant: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// A new avatar for an agent. Replaces the agent's current avatar.
#[derive(Clone, Debug)]
pub struct SetAgentAvatar {
    pub org_id: i64,
    pub agent_id: Uuid,
    /// Where the image came from. Only `upload` today; a curated predefined
    /// set will add a value here.
    pub source: String,
    pub variants: Vec<AgentAvatarVariantInput>,
}

/// One stored variant, as served.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AgentAvatarVariantRow {
    pub content_type: String,
    pub data: Vec<u8>,
}
