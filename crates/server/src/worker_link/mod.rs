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
