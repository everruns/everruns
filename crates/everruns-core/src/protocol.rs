// Agent Protocol - Re-exports for backwards compatibility
//
// This module re-exports atoms and loop types for convenience.
// For new code, prefer importing directly from `atoms` and `loop` modules.

// Re-export everything from atoms
pub use crate::atoms::{
    AddUserMessageAtom, AddUserMessageInput, AddUserMessageResult, Atom, CallModelAtom,
    CallModelInput, CallModelResult, ExecuteToolAtom, ExecuteToolInput, ExecuteToolResult,
};

// Re-export from loop
pub use crate::r#loop::{AgentLoop2, LoadMessagesResult};

// Backwards compatibility alias
pub type AgentProtocol<M, L, T> = AgentLoop2<M, L, T>;
