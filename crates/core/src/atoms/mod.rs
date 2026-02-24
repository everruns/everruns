// Atomic Operations for Agent Protocol
//
// Atoms are self-contained, stateless operations that can be composed
// to build agent loops. Each atom handles its own message storage.
//
// Key concepts:
// - Atom trait: Defines atomic operations with Input → Output
// - AtomContext: Contains session_id, turn_id, input_message_id, exec_id
// - Each Atom handles: load messages → execute → store results
// - Stateless: No internal state, all state passed in/out
// - Composable: Atoms can be orchestrated by external systems (durable execution, custom loops)
// - Event emission: Atoms emit events for observability

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::typed_id::{ExecId, MessageId, SessionId, TurnId};

// ----------------------------------------------------------------------------
// Atom Modules
// ----------------------------------------------------------------------------

// Turn-based atoms for the turn workflow
mod act;
mod input;
mod reason;

// Re-export atoms and their types
pub use act::{ActAtom, ActInput, ActResult, ToolCallResult};
pub use input::{InputAtom, InputAtomInput, InputAtomResult};
pub use reason::{ReasonAtom, ReasonInput, ReasonResult};

// ----------------------------------------------------------------------------
// AtomContext - Runtime context for atom execution
// ----------------------------------------------------------------------------

/// Context for atom execution within a turn
///
/// AtomContext provides the execution context for atoms, including:
/// - session_id: The session this turn belongs to
/// - turn_id: Unique identifier for the current turn (conversation round)
/// - input_message_id: The ID of the input message that triggered this turn
/// - exec_id: Unique identifier for this specific atom execution (also serves as version)
///
/// This context is passed to all atoms during execution and enables:
/// - Tracking execution lineage
/// - Correlating events across atom executions
/// - Supporting cancellation and resumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomContext {
    /// Session ID - the conversation session
    pub session_id: SessionId,

    /// Turn ID - unique identifier for the current turn (user input → final response)
    /// Uses typed TurnId for type safety and consistent prefixed format.
    pub turn_id: TurnId,

    /// Input message ID - the user message that triggered this turn
    pub input_message_id: MessageId,

    /// Execution ID - unique identifier for this specific atom execution
    /// Also serves as a version identifier for the execution
    pub exec_id: ExecId,
}

impl AtomContext {
    /// Create a new AtomContext
    pub fn new(session_id: SessionId, turn_id: TurnId, input_message_id: MessageId) -> Self {
        Self {
            session_id,
            turn_id,
            input_message_id,
            exec_id: ExecId::new(),
        }
    }

    /// Create a new execution context for a new atom within the same turn
    pub fn next_exec(&self) -> Self {
        Self {
            session_id: self.session_id,
            turn_id: self.turn_id,
            input_message_id: self.input_message_id,
            exec_id: ExecId::new(),
        }
    }
}

// ----------------------------------------------------------------------------
// Atom Trait - Core abstraction for atomic operations
// ----------------------------------------------------------------------------

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
/// - Durable activity integration
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

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atom_context_new() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        let context = AtomContext::new(session_id, turn_id, input_message_id);

        assert_eq!(context.session_id, session_id);
        assert_eq!(context.turn_id, turn_id);
        assert_eq!(context.input_message_id, input_message_id);
        // exec_id should be auto-generated
        assert!(!context.exec_id.uuid().is_nil());
    }

    #[test]
    fn test_atom_context_next_exec() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        let context1 = AtomContext::new(session_id, turn_id, input_message_id);
        let context2 = context1.next_exec();

        // Same session, turn, and input_message_id
        assert_eq!(context2.session_id, context1.session_id);
        assert_eq!(context2.turn_id, context1.turn_id);
        assert_eq!(context2.input_message_id, context1.input_message_id);
        // Different exec_id
        assert_ne!(context2.exec_id, context1.exec_id);
    }

    #[test]
    fn test_atom_context_serialization() {
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        let json = serde_json::to_string(&context).unwrap();
        let parsed: AtomContext = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, context.session_id);
        assert_eq!(parsed.turn_id, context.turn_id);
        assert_eq!(parsed.input_message_id, context.input_message_id);
        assert_eq!(parsed.exec_id, context.exec_id);
    }
}
