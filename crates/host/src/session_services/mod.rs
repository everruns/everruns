//! Backend-neutral session service contracts and capability implementations.
//!
//! This module is the dependency-inversion seam between execution hosts and the
//! Everruns control plane. It contains only turn-time session services: no
//! organizations, billing, connectors, audit records, email clients, or HTTP
//! transport.
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```
//! use everruns_host::session_services::SESSION_CAPABILITY_ID;
//!
//! assert_eq!(SESSION_CAPABILITY_ID, "session");
//! ```

pub mod capabilities;
pub mod session_mutator;

pub use capabilities::{
    GetSessionInfoTool, KvStoreTool, SESSION_CAPABILITY_ID, SESSION_STORAGE_CAPABILITY_ID,
    SecretStoreTool, SessionCapability, SessionCapabilityConfig, SessionStorageCapability,
    SessionTitleMutation, WriteSessionTitleTool, is_internal_session_kv_key,
    is_internal_session_secret_name, session_title_updated_event, update_session_title_with_event,
};
pub use session_mutator::{SessionMutator, SessionMutatorExt};
