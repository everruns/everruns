// Goal capability
//
// Decision: durable per-session "goal" state, mirroring Codex CLI's /goal.
// The user sets a long-running objective via `/goal <text>`; the agent reads
// it back from the session VFS each turn and works toward it. Lifecycle
// subcommands (show/pause/resume/clear) keep the goal user-owned.
//
// Decision: goal state lives in the session VFS at
// `.everruns/{session_id}/goal.json` (agent-visible as
// `/workspace/.everruns/{session_id}/goal.json`). VFS storage gives us
// agent-discoverable, user-editable state without a DB migration. The
// session_id in the path is redundant inside the VFS (one VFS per session)
// but keeps the file self-describing if it ever surfaces outside that
// context (exports, logs, snapshots) and reserves
// `.everruns/{session_id}/` as a namespace for future capability-owned
// state (journals, memory, …).

use super::{Capability, CapabilityStatus, SystemPromptContext};
use crate::command::{CommandArg, CommandDescriptor, CommandSource};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub const GOAL_CAPABILITY_ID: &str = "goal";

/// Max objective length, matching Codex's 4_000-char ceiling.
pub const MAX_GOAL_OBJECTIVE_LEN: usize = 4_000;

/// File-store-relative directory for goal state (no `/workspace` prefix).
pub fn goal_dir(session_id: SessionId) -> String {
    format!(".everruns/{session_id}")
}

/// File-store-relative path to the goal JSON.
pub fn goal_file_path(session_id: SessionId) -> String {
    format!("{}/goal.json", goal_dir(session_id))
}

/// Agent-visible path (under `/workspace`) used in system-prompt copy.
pub fn agent_visible_goal_path(session_id: SessionId) -> String {
    format!("/workspace/.everruns/{session_id}/goal.json")
}

/// Lifecycle state for a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
    Cleared,
}

/// Persisted goal record.
///
/// Schema is intentionally small and forward-compatible: extra fields added
/// by future capability versions are preserved by `serde(default)` on the
/// reader side. Optimistic concurrency uses `version`, incremented on every
/// write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRecord {
    /// User-set objective text.
    pub objective: String,
    /// Current lifecycle state.
    pub status: GoalStatus,
    /// Monotonic version, incremented on every write.
    #[serde(default)]
    pub version: u64,
    /// ISO-8601 timestamp when the goal was first set.
    pub set_at: String,
    /// ISO-8601 timestamp of the last status/objective change.
    pub updated_at: String,
}

pub struct GoalCapability;

#[async_trait]
impl Capability for GoalCapability {
    fn id(&self) -> &str {
        GOAL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Goal"
    }

    fn description(&self) -> &str {
        "Long-running session objective with /goal lifecycle commands."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("target")
    }

    fn category(&self) -> Option<&str> {
        Some("Productivity")
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "goal".to_string(),
            description:
                "Set, view, pause, resume, or clear a long-running objective for this session."
                    .to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "subcommand_or_objective".to_string(),
                description: "Subcommand (show, pause, resume, clear) or new objective text."
                    .to_string(),
                required: false,
            }],
        }]
    }

    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let file_store = ctx.file_store.as_ref()?;
        let path = goal_file_path(ctx.session_id);
        let file = match file_store.read_file(ctx.session_id, &path).await {
            Ok(Some(f)) => f,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %ctx.session_id,
                    "Failed to read goal file, skipping system-prompt contribution"
                );
                return None;
            }
        };
        let content = file.content.as_deref()?;
        let record: GoalRecord = serde_json::from_str(content).ok()?;
        if record.status != GoalStatus::Active {
            return None;
        }
        let visible_path = agent_visible_goal_path(ctx.session_id);
        Some(format!(
            "<capability id=\"{GOAL_CAPABILITY_ID}\">\n# Active Session Goal\n\nThis session has an active long-running goal:\n\n> {objective}\n\nThe canonical goal record lives at `{visible_path}`. Treat it as durable context across turns: re-read it whenever you need to confirm scope, and bias your work toward making concrete progress on it. The user controls the goal via `/goal` (show / pause / resume / clear); do not edit the file yourself unless explicitly asked.\n</capability>",
            objective = record.objective,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use uuid::Uuid;

    fn fixture_session_id() -> SessionId {
        SessionId::from(Uuid::nil())
    }

    #[test]
    fn capability_metadata() {
        let cap = GoalCapability;
        assert_eq!(cap.id(), GOAL_CAPABILITY_ID);
        assert_eq!(cap.name(), "Goal");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("target"));
        assert_eq!(cap.category(), Some("Productivity"));
    }

    #[test]
    fn registers_goal_command_with_optional_arg() {
        let cap = GoalCapability;
        let commands = cap.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "goal");
        assert_eq!(commands[0].source, CommandSource::System);
        assert_eq!(commands[0].args.len(), 1);
        assert!(!commands[0].args[0].required);
    }

    #[test]
    fn path_helpers_include_session_id() {
        let sid = fixture_session_id();
        let dir = goal_dir(sid);
        let file = goal_file_path(sid);
        let visible = agent_visible_goal_path(sid);
        assert!(dir.starts_with(".everruns/"));
        assert!(dir.contains(&sid.to_string()));
        assert!(file.ends_with("/goal.json"));
        assert!(visible.starts_with("/workspace/.everruns/"));
        assert!(visible.contains(&sid.to_string()));
    }

    #[test]
    fn status_serialises_snake_case() {
        let s = serde_json::to_string(&GoalStatus::Active).unwrap();
        assert_eq!(s, "\"active\"");
        let s = serde_json::to_string(&GoalStatus::Paused).unwrap();
        assert_eq!(s, "\"paused\"");
    }

    #[test]
    fn record_roundtrips() {
        let record = GoalRecord {
            objective: "ship it".to_string(),
            status: GoalStatus::Active,
            version: 1,
            set_at: "2026-05-20T00:00:00Z".to_string(),
            updated_at: "2026-05-20T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: GoalRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objective, "ship it");
        assert_eq!(back.status, GoalStatus::Active);
        assert_eq!(back.version, 1);
    }
}
