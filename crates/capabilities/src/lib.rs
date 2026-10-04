//! Hosted capabilities and sandbox orchestration for [Everruns](https://everruns.com).
//! Hosts use portable runtime definitions and `everruns-contracts` service interfaces.
//! Control-plane persistence and API records belong to the server.
//!
//! ```
//! use everruns_capabilities::capabilities::hosted_capability_registry;
//! let _registry = hosted_capability_registry();
//! ```

pub mod background_run;
pub mod capabilities;
pub mod channel_message_sender;
pub mod connector;
#[cfg(feature = "container-sandbox")]
pub mod container_sandbox;
pub mod host_extension;
pub mod knowledge_store;
pub mod memory;
pub mod platform_store;
pub mod sandbox_checkpoint;
pub mod sandbox_state;
pub mod session_sandbox;
pub mod session_sqldb;
pub mod vector_store;
pub use everruns_contracts::slack_action;
pub use everruns_contracts::slack_action::{
    SlackAction, SlackActionError, SlackActionInvoker, SlackActionInvokerExt, SlackActionOutcome,
};
pub use everruns_host::session_services::session_mutator;
pub use everruns_host::{SessionMutator, SessionMutatorExt};

pub use connector::{
    Connector, ConnectorFormSchema, ConnectorPlugin, ConnectorRegistry, ConnectorRegistryBuilder,
    ConnectorType, ConnectorValidation, FieldType, FormField,
};
pub use host_extension::{PlatformHostBackendsExt, PlatformStoreFactory, PlatformToolAugmentor};
pub use knowledge_store::{
    KnowledgeIndexSearchExt, KnowledgeIndexSearchHit, KnowledgeSearchHit, KnowledgeStore,
    KnowledgeStoreExt,
};
pub use memory::{
    MemoryConfig, MemoryMountAccess, MemoryMountConfig, validate_memory_config,
    validate_mount_config_shape,
};
pub use platform_store::{
    PlatformCreateSessionRequest, PlatformMessage, PlatformStore, PlatformStoreExt,
    PlatformStoreSubagentDelegate,
};
pub use sandbox_checkpoint::{
    DurableToolResultStoreExt, MAX_CHECKPOINT_COLLECT_LIMIT, NewSandboxCheckpoint,
    SandboxCheckpoint, SandboxCheckpointError, SandboxCheckpointKind, SandboxCheckpointStore,
    SandboxCheckpointStoreExt, SandboxRef,
};
pub use session_sandbox::{
    DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS, SESSION_SANDBOX_CAPABILITY_ID,
    SESSION_SANDBOX_SECRET_NAME, SessionSandboxConfig, SessionSandboxExecRequest,
    SessionSandboxExecResponse, SessionSandboxInitConfig, SessionSandboxInstance,
    SessionSandboxProvider, SessionSandboxProviderPlugin, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse, create_session_sandbox_provider, delete_session_sandbox,
    delete_session_sandbox_state, ensure_session_sandbox_running, load_session_sandbox_state,
    pause_session_sandbox, run_session_sandbox_init_if_needed, save_session_sandbox_state,
    session_sandbox_config_from_capabilities, session_sandbox_tool_hints,
};
pub use session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SessionSqlDbStoreExt,
    SqlExecuteResult, SqlQueryResult, TableSchema,
};
pub use vector_store::{
    EmbeddingCallUsage, InMemoryVectorStore, KnowledgeIndexCitation, KnowledgeIndexSearch,
    KnowledgeIndexSearchOutcome, VectorMatch, VectorQuery, VectorRecord, VectorStore,
    VectorStoreExt, index_namespace,
};

pub use sandbox_state::{
    SandboxPersistenceStore, SandboxStateError, SandboxStateStore, SandboxStateStoreExt,
};
