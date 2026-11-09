//! Neutral tool-narration request types.
//!
//! Narration is the short, human-readable line a client shows for a tool call
//! ("Read AGENTS.md", "Searching the web for rust"). These types carry no
//! runtime dependency so both the engine's `runtime::tool_narration` helpers
//! and the dependency-light `capability::definition` authoring surface can
//! share them; capability packages narrate without enabling the runtime SPI.

use serde_json::Value;

/// The lifecycle point a narration line describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolNarrationPhase {
    /// The call is about to run.
    Started,
    /// The call is waiting, e.g. for approval or a detached result.
    Waiting,
    /// The call finished successfully.
    Completed,
    /// The call failed.
    Failed,
}

/// One narration request for an application-authored tool.
///
/// Passed to `capability::Handler::narrate`, `FunctionTool::narrate`, and
/// `#[everruns::tool(narrate = …)]`. Returning `None` from any of them keeps
/// the engine's generic, localized fallback ("Running {display name}: …").
///
/// Arguments are exactly what the model sent, so treat them as untrusted:
/// render only fields you chose, bounded (see the `truncate` and
/// `safe_arg_str` helpers in `everruns::narration`), and never secrets.
#[derive(Debug, Clone, Copy)]
pub struct ToolNarration<'a> {
    arguments: &'a Value,
    phase: ToolNarrationPhase,
    locale: Option<&'a str>,
    result: Option<&'a Value>,
}

impl<'a> ToolNarration<'a> {
    /// A request for `arguments` at `phase`, in the resolved session `locale`.
    pub fn new(arguments: &'a Value, phase: ToolNarrationPhase, locale: Option<&'a str>) -> Self {
        Self {
            arguments,
            phase,
            locale,
            result: None,
        }
    }

    /// Attach the call's model-visible result (completed phase).
    pub fn with_result(mut self, result: Option<&'a Value>) -> Self {
        self.result = result;
        self
    }

    /// The call's raw JSON arguments.
    pub fn arguments(&self) -> &'a Value {
        self.arguments
    }

    /// The lifecycle point being narrated.
    pub fn phase(&self) -> ToolNarrationPhase {
        self.phase
    }

    /// The resolved session locale (for example `"uk"`), when known.
    pub fn locale(&self) -> Option<&'a str> {
        self.locale
    }

    /// The call's model-visible result, set for the completed phase.
    pub fn result(&self) -> Option<&'a Value> {
        self.result
    }
}
