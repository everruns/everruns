//! The server's side of the worker link: the internal gRPC service remote
//! workers call, and the direct adapters an in-process worker uses instead.

// Direct worker adapters for in-process task worker
pub mod direct_worker_adapters;

// Internal gRPC service for worker communication
pub mod grpc_service;

// Catalog-backed platform tools (MCP and the Platform capability) and the
// portable delegation projection runtime callers opt into.
pub mod platform_command_surface;
pub(crate) mod runtime_command_view;

pub(crate) mod consumer_delegation;

// Restricted customer-hosted execution, separate from the trusted worker link.
pub mod custom_workers;

/// The refusal a run with no person behind it gets for a command that acts
/// for a person. Agents read it as a tool error (`AgentLoopError::Refused`),
/// so it says what happened in their terms and nothing internal.
pub(crate) const NO_PERSON_REFUSAL: &str = "This run has no person behind it (a schedule, \
     webhook or event started it), so it cannot use commands that act for a person. Do the \
     work without this command, or tell the person who set up this run that it needs it.";
