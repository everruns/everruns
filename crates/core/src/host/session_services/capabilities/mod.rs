//! Standard capabilities backed by neutral session services.

pub mod session;
pub mod session_storage;

pub use session::{
    GetSessionInfoTool, SESSION_CAPABILITY_ID, SessionCapability, SessionCapabilityConfig,
    SessionTitleMutation, WriteSessionTitleTool, session_title_updated_event,
    update_session_title_with_event,
};
pub use session_storage::{
    KvStoreTool, SESSION_STORAGE_CAPABILITY_ID, SecretStoreTool, SessionStorageCapability,
    is_internal_session_kv_key, is_internal_session_secret_name,
};
