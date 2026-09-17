#![deny(missing_docs)]

//! The application-facing crate for the [Everruns Framework](https://docs.everruns.com/framework/).
//!
//! Build agents, attach provider configuration, select model ids, add typed
//! tools, run isolated multi-turn sessions, bind backend-owned workspace heads
//! through Environments, read bounded history, resume typed
//! session identities, observe events, cancel work, and inspect the next model context
//! without constructing an execution host. Default features stay offline; the
//! deterministic simulator needs no credentials or network.
//! The default registry advertises only portable capabilities; hosted
//! knowledge, delegation, task, hook, and platform-management implementations
//! require an explicit Everruns Platform host.
//!
//! `everruns` is the primary Rust library in the
//! [Everruns](https://everruns.com) ecosystem. Ordinary applications begin
//! here; advanced execution hosts use `everruns` plus
//! [`everruns-host`](https://docs.rs/everruns-host) and focused sibling crates.
//!
//! # Example
//!
//! Run one simulated turn, importing only `everruns`:
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::{Agent, Engine, Model};
//!
//! let agent = Agent::builder()
//!     .instructions("You are a helpful assistant.")
//!     .model(Model::simulated("4"))
//!     .build()
//!     .expect("valid agent");
//! let engine = Engine::new();
//! let result = engine.create(agent).send_and_wait("What is 2 + 2?").await?;
//! assert!(result.success);
//! assert_eq!(result.response, "4");
//! # Ok(())
//! # }
//! ```

// Let code emitted by `#[everruns::tool]` resolve `::everruns::…` paths even
// inside this crate's own tests and doctests.
extern crate self as everruns;

// --- Value-first agent description and execution -------------------------
mod agent;
#[cfg(feature = "capabilities")]
pub mod capability;
mod capability_config;
/// Stability: alpha — may change without a major bump; see [`stability`].
pub mod classifier;
mod context;
mod default_workspace;
mod engine;
mod events;
mod history;
mod hooks;
/// Stability: stable — no breaking change without a major bump; see [`stability`].
pub mod llm;
mod mcp;
/// Stability: alpha — may change without a major bump; see [`stability`].
pub mod models;
mod plugin;
mod session;
/// Stability tiers and the marking convention.
pub mod stability;
mod tool;
/// Session-owned background work, scheduling, cancellation, and wakes.
pub mod work;
pub use agent::{Agent, AgentBuilder, BuildError, Model};
pub use capability_config::{CapabilityRef, CapabilitySpec, IntoCapability};
pub use classifier::{Answers, Classification, Classifier, ClassifierError};
pub use context::{ContextMessage, SessionContext, ToolInfo};
pub use engine::{Engine, InMemoryEngine};
pub use events::{
    CancellationToken, EVENT_STREAM_CAPACITY, EventStream, EventStreamError, RunOptions,
    SessionEvent, SessionEventKind,
};
#[cfg(feature = "builtins")]
pub use everruns_builtins::{
    AgentInstructionsConfig, CompactionConfig, CompactionStrategy, Skills, StatelessTodoList,
    ToolSearch,
};
pub use everruns_core::classifier::{
    ClassificationAnswer, ClassificationOutcome, ClassificationQuestion, ClassificationRequest,
    ClassifierService,
};
#[deprecated(note = "use WorkspaceBackend")]
pub use everruns_host::WorkspaceBackend as WorkspaceProvider;
#[deprecated(note = "use WorkspaceBackendId")]
pub use everruns_host::WorkspaceBackendId as WorkspaceProviderId;
pub use everruns_host::{
    Compute, ComputeCapabilities, ComputeError, ComputeKind, ComputeSession, Containment,
    ContainmentLevel, Durability, EnvironmentError, ExecRequest, ExecResult, NetworkPolicy,
};
#[cfg(feature = "host-compute")]
pub use everruns_host::{HostCompute, HostComputeSession};
#[cfg(feature = "bashkit")]
pub use everruns_integrations_bashkit::BashkitShell;
#[cfg(feature = "duckduckgo")]
pub use everruns_integrations_duckduckgo::DuckDuckGo;
#[cfg(feature = "filesystem")]
pub use everruns_integrations_filesystem::FileSystem;
/// The TypeSafe-backed classifier, for [`Classifier::new`], and the capability
/// that hands the same tool to an agent.
#[cfg(feature = "jev")]
pub use everruns_integrations_typesafe::{Jev, TypeSafeClassifier};
#[cfg(feature = "web-fetch")]
pub use everruns_integrations_web_fetch::WebFetch;
pub use history::{
    HistoryCursor, HistoryCursorParseError, HistoryError, HistoryPage, HistoryPages, HistoryQuery,
    ResumeError, SessionMessage,
};
pub use hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookPoint, IntoHookResult, ToolEndContext,
    ToolStartContext, TurnStartContext,
};
pub use llm::{Completion, CompletionError};
pub use mcp::McpServer;
pub use models::{CatalogError, ModelInfo};
pub use plugin::PluginError;
pub use session::{
    CancelError, EnvironmentSessionBuilder, RunError, SendDisposition, SentMessage, Session,
    SessionEnvironmentError, Turn, TurnHandle,
};
pub use tool::{FunctionTool, IntoTool, IntoToolResult, Tool, ToolResponse};

#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "local")]
#[deprecated(note = "use LocalGitWorkspace")]
pub use local::LocalGitWorkspace as LocalGitWorkspaceProvider;
#[cfg(feature = "local")]
pub use local::{LocalConfig, LocalGitWorkspace};

#[cfg(all(test, feature = "macros"))]
mod tool_macro_tests;

// --- Function-tool procedural macro (feature-gated) ---------------------
/// Turn a typed async function into an agent tool.
///
/// `#[everruns::tool]` generates the argument JSON Schema and adapter for a
/// plain async function so it can be handed to
/// [`AgentBuilder::tool`](crate::AgentBuilder::tool) without writing either by
/// hand. See the [`macro@tool`] documentation for supported signatures and
/// options. Requires the default-enabled `macros` feature.
#[cfg(feature = "macros")]
pub use everruns_macros::tool;

/// Runtime support for code emitted by [`macro@tool`]. Not a stable API — the
/// expansion references these items by path so the calling crate needs no
/// direct dependency on `serde`, `schemars`, or `serde_json`.
#[cfg(feature = "macros")]
#[doc(hidden)]
pub mod __macro_support {
    pub use schemars;
    pub use serde;
    pub use serde_json::{self, Value};

    /// Serialize a type's JSON Schema to a `Value` for `FunctionTool::new`.
    pub fn schema_for<T: schemars::JsonSchema>() -> Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
    }

    /// Deserialize the model's call arguments into the generated struct.
    pub fn from_value<T: serde::de::DeserializeOwned>(
        value: Value,
    ) -> Result<T, serde_json::Error> {
        serde_json::from_value(value)
    }

    /// Serialize a handler's success value to a `Value`.
    pub fn to_value<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
        serde_json::to_value(value)
    }
}

// --- Real LLM provider configuration (feature-gated) --------------------
// The default facade build stays offline; provider modules compile only when
// their feature is enabled. `openai` adds `providers::openai::OpenAI`.
pub mod providers;
#[cfg(feature = "openai")]
pub use providers::openai::{OpenAI, OpenAIError};

// --- Runtime construction and execution ---------------------------------
// Note: the value-first `AgentBuilder` above intentionally replaces the
// low-level host `AgentBuilder` at the facade root. Advanced hosts that need
// the low-level builders depend on `everruns-host` directly.
pub use everruns_host::{
    Environment, EnvironmentBuilder, Workspace, WorkspaceBackend, WorkspaceBackendId,
    WorkspaceBinding, WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff, WorkspaceError,
    WorkspaceHead, WorkspaceHeadAccess, WorkspaceHeadBuilder, WorkspaceHeadDescriptor,
    WorkspaceHeadId, WorkspaceHeadRequest, WorkspaceHeadResource, WorkspaceHeadStatus,
};

// --- Portable message, model, and platform types ------------------------
pub use everruns_core::turn::TurnStopReason;
pub use everruns_core::{
    ContentPart, Controls, ImageContentPart, InitialFile, InputMessage, MessageRole,
    ReasoningConfig, WorkspacePolicy, WorkspacePolicyBuilder, WorkspacePolicyError,
};
pub use everruns_provider::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCallConfigBuilder, LlmCompletionMetadata, LlmContentPart,
    LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponse, LlmResponseStream, LlmStreamEvent,
};
// Reasoning is part of the public surface: `ReasoningConfig` above carries a
// `ReasoningEffort`, and the artifact types appear on assistant messages.
pub use everruns_provider::model::ReasoningEffort;
// Model identity and capability metadata: what a picker renders next to an id,
// and what an application checks before selecting one.
pub use everruns_provider::model::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor,
};
pub use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
pub use everruns_provider::{ExecutionPhase, PhaseSource};
// Required by the public `ChatDriver` SPI and runtime error contract:
// downstream consumers can inspect provider failures without depending on an
// implementation crate.
pub use everruns_provider::error::{AgentLoopError, BillingPressureReason, LlmError, LlmErrorKind};
pub use everruns_provider::runtime_provider::{
    BearerAuth, Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint, ProviderKey,
    StaticHeaderAuth,
};
// Credential resolution. A driver declares which environment variables it reads
// on its own descriptor, following its vendor's SDK; `EnvCredentialProvider` is
// the one place that pairs those declarations with the process environment, and
// is for standalone/CLI/dev use only.
pub use everruns_provider::credential_provider::{
    CredentialProvider, EnvCredentialError, EnvCredentialProvider, ProviderCredentials,
    provider_from_env,
};
pub use everruns_provider::credential_schema::{CredentialFormSchema, FieldType, FormField};
pub use everruns_provider::driver_registry::{
    BoxedChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverRegistry,
};
pub use everruns_provider::provider::DriverId;
pub use everruns_provider::tool_types::{ToolCall, ToolDefinition};
pub use everruns_provider::typed_id::{SessionId, WorkspaceId};

// --- Deterministic in-process LLM simulator -----------------------------
pub use everruns_llmsim::LlmSimConfig;

/// The common path: everything needed to describe an agent and run turns.
///
/// ```
/// use everruns::prelude::*;
///
/// let agent = Agent::builder()
///     .instructions("You are concise.")
///     .model(Model::simulated("Sure."))
///     .build();
/// assert!(agent.is_ok());
/// ```
pub mod prelude {
    #[cfg(feature = "bashkit")]
    pub use crate::BashkitShell;
    #[cfg(feature = "duckduckgo")]
    pub use crate::DuckDuckGo;
    #[cfg(feature = "filesystem")]
    pub use crate::FileSystem;
    #[cfg(feature = "local")]
    pub use crate::LocalConfig;
    #[cfg(feature = "local")]
    pub use crate::LocalGitWorkspace;
    #[cfg(feature = "local")]
    #[allow(deprecated)]
    #[deprecated(note = "use LocalGitWorkspace")]
    pub use crate::LocalGitWorkspaceProvider;
    #[cfg(feature = "web-fetch")]
    pub use crate::WebFetch;
    #[cfg(feature = "capabilities")]
    pub use crate::capability;
    pub use crate::models;
    #[cfg(feature = "macros")]
    pub use crate::tool;
    pub use crate::work::{
        TaskOutcome, TaskRequest, WakePolicy, WakeRequest, WorkQueue, WorkSchedule,
    };
    pub use crate::{
        Agent, AgentBuilder, AgentStartContext, Answers, BuildError, CancelError,
        CancellationToken, CapabilityRef, CapabilitySpec, Classification, Classifier,
        ClassifierError, Completion, CompletionContext, CompletionError, Engine, Environment,
        EventStream, EventStreamError, FunctionTool, HistoryCursor, HistoryCursorParseError,
        HistoryError, HistoryPage, HistoryPages, HistoryQuery, HookFailure, HookPoint,
        InMemoryEngine, InitialFile, IntoCapability, IntoHookResult, IntoTool, IntoToolResult,
        LlmSimConfig, McpServer, Model, PluginError, ResumeError, RunError, RunOptions,
        SendDisposition, SentMessage, Session, SessionContext, SessionEnvironmentError,
        SessionEvent, SessionEventKind, SessionId, SessionMessage, Tool, ToolEndContext, ToolInfo,
        ToolResponse, ToolStartContext, Turn, TurnHandle, TurnStartContext, Workspace,
        WorkspaceBackend, WorkspaceBackendId, WorkspaceDiff, WorkspaceError, WorkspaceHead,
        WorkspaceHeadAccess, WorkspaceHeadId, WorkspaceId, WorkspacePolicy, WorkspacePolicyBuilder,
        WorkspacePolicyError,
    };
    #[cfg(feature = "builtins")]
    pub use crate::{
        AgentInstructionsConfig, CompactionConfig, CompactionStrategy, Skills, StatelessTodoList,
        ToolSearch,
    };
    pub use crate::{CatalogError, ModelInfo, ModelProfile};
    pub use crate::{DriverId, EnvCredentialError, EnvCredentialProvider};
    #[cfg(feature = "openai")]
    pub use crate::{OpenAI, OpenAIError};
    #[allow(deprecated)]
    #[deprecated(note = "use WorkspaceBackend and WorkspaceBackendId")]
    pub use crate::{WorkspaceProvider, WorkspaceProviderId};
    pub use everruns_core::turn::TurnStopReason;
    pub use everruns_core::{ContentPart, InputMessage, MessageRole};
}
