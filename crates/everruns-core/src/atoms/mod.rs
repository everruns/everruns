// Atomic Operations for Agent Protocol
//
// Atoms are self-contained, stateless operations that can be composed
// to build agent loops. Each atom handles its own message storage.
//
// Key concepts:
// - Atom trait: Defines atomic operations with Input → Output
// - Each Atom handles: load messages → execute → store results
// - Stateless: No internal state, all state passed in/out
// - Composable: Atoms can be orchestrated by external systems (Temporal, custom loops)

use async_trait::async_trait;

use crate::error::Result;

// ============================================================================
// Atom Modules
// ============================================================================

mod add_user_message;
mod call_model;
mod execute_tool;

// Re-export atoms and their types
pub use add_user_message::{AddUserMessageAtom, AddUserMessageInput, AddUserMessageResult};
pub use call_model::{CallModelAtom, CallModelInput, CallModelResult};
pub use execute_tool::{ExecuteToolAtom, ExecuteToolInput, ExecuteToolResult};

// ============================================================================
// Atom Trait - Core abstraction for atomic operations
// ============================================================================

/// An atomic operation in the agent protocol
///
/// Atoms are self-contained operations that:
/// 1. Take an input with all required context
/// 2. Perform their operation (may load/store messages)
/// 3. Return a result
///
/// This trait enables:
/// - Uniform execution interface for all operations
/// - Easy composition and orchestration
/// - Temporal activity integration
/// - Testing and mocking
#[async_trait]
pub trait Atom: Send + Sync {
    /// Input type for this atom
    type Input: Send;
    /// Output type for this atom
    type Output: Send;

    /// Name of this atom (for logging/debugging)
    fn name(&self) -> &'static str;

    /// Execute the atom with the given input
    async fn execute(&self, input: Self::Input) -> Result<Self::Output>;
}
