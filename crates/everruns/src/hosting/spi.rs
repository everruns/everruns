//! Storage and ownership contracts for implementing [`super::SessionHosting`].
//!
//! These are the supported Framework paths for host implementers. Their type
//! identity is shared with Core, but hosts need no direct Core dependency. This
//! surface is experimental alongside session hosting. It exposes storage ports,
//! not the actor driver, planner, turn loop or runtime construction.
//!
//! Use one authority for admission, journals, receipts and epochs. Reader ports
//! must refuse mutations; executor ports must check an unexpired epoch inside
//! every mutation transaction. In-memory defaults in [`HostBackends`] are only
//! suitable for reconstructed definitions and explicitly volatile services.

/// Errors returned by authority and storage operations.
pub mod error {
    pub use everruns_contracts::error::{AgentLoopError, Result};
}
/// Typed authority identities.
pub mod ids {
    pub use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
}
/// Canonical event values shared by the authority and Engine.
pub mod events {
    pub use everruns_core::events::{
        Event, EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
        ToolCompletedData,
    };
}
/// Admission payloads and retained conversation values.
pub mod message {
    pub use everruns_core::{RuntimeMessage, RuntimeMessageRole};
}
/// Epoch ownership, renewal, release and bounded journal reads.
pub mod actor {
    pub use everruns_core::host::session_actor::store::{
        ActorFence, ActorJournal, ActorLeaseLost, ActorRelease, ActorRenewal, LeasedActor,
        SessionActorStore, actor_lease_lost, is_actor_lease_lost,
    };
}
/// Read the retained execution view; Engine supplies executable definitions.
pub mod loading {
    pub use everruns_core::execution_loading::SessionStore;
}
/// Session-scoped values and secrets; unsupported services must fail explicitly.
pub mod storage {
    pub use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
}
/// Durable approval request interpretation shared with Engine.
pub mod approval {
    pub use everruns_contracts::tool_approval_types::ToolApprovalRequired;
}
pub use everruns_contracts::ASK_USER_TOOL_NAME;
pub use everruns_core::host::{
    AcceptedTurnInput, EventCursor, EventDurability, EventLog, EventLogError, EventPage,
    EventReadLimit, EventReadRequest, EventReader, HostBackends, RuntimeSessionStore,
    SessionMutator,
};
/// Shared command reducer; execute inside the authority transaction, including remote hosts.
pub use everruns_core::session_commands;
pub use everruns_core::{
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore, ExecutionSession,
    ProactiveCompactionAttempt, RuntimeMessage,
};

/// Adapt isolate-local values/futures to the shared Send/Sync host contracts.
/// Only valid in a single-threaded Wasm embedding; never move JavaScript values
/// between threads. Native hosts must use genuinely Send/Sync transports.
#[cfg(target_arch = "wasm32")]
pub use everruns_contracts::rt::AssertSend;
