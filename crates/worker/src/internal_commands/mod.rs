//! Runtime stores served by the control plane's internal domain commands.
//!
//! Decision: a worker operation that is neither hot nor streaming is a domain
//! command on the server (`INTERNAL_COMMAND_PATH_PREFIX` keeps it off public
//! surfaces), and the runtime-facing store for it is written once, here, over
//! [`InternalCommandTransport`]. A gRPC worker carries the call over
//! `ExecuteCommand`; the server's in-process worker dispatches it directly.
//! Neither side keeps per-operation code: no bespoke RPC, no handler, no
//! direct adapter. Hot and streaming operations (event emission, turn context,
//! durable task claims and heartbeats) keep their dedicated RPCs.
//!
//! Failures cross as `proto::CommandError` on both transports, so a store maps
//! one vocabulary whichever way it was reached.

pub use everruns_internal_protocol::commands::{
    CommandConnectionResolver, CommandLeasedResourceStore, CommandOrgEgressAllowlist,
    CommandSessionResourceRegistry, InternalCommandTransport,
};
use everruns_internal_protocol::commands::{call, call_secret, decode, failure, is_bad_request};

mod credentials;
pub mod dreaming;
pub use everruns_internal_protocol::commands::knowledge;
mod poppy;
mod session_schedules;
mod session_storage;
mod session_tasks;

pub use poppy::CommandPoppyInvoker;
pub use session_schedules::CommandSessionScheduleStore;
pub use session_storage::CommandSessionStorageStore;
pub use session_tasks::CommandSessionTaskRegistry;
