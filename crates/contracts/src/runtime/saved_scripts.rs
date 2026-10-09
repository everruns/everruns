//! Saved scripts: shell scripts an agent owns and calls from its shell as
//! `tools scripts <name>` (Tools in Shell D8).
//!
//! Decision: the host binds the store to the session's agent before the turn
//! starts, so a script is always listed and saved for the agent whose turn it
//! is, and a tool never names an agent. The host also decides whether this
//! turn may save scripts (the capability's `manage_scripts` config), so the
//! shell does not read capability config.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One saved script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedScript {
    /// Lowercase name, unique per agent: `^[a-z][a-z0-9_-]{0,63}$`.
    pub name: String,
    /// One line on what it does, shown in `tools scripts`.
    pub description: String,
    /// JSON Schema for the input object, when the script declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// The shell script.
    pub body: String,
}

/// The current agent's saved scripts. Errors are messages a script can show.
#[async_trait]
pub trait SavedScriptStore: Send + Sync {
    /// Every active script of the agent.
    async fn list(&self) -> Result<Vec<SavedScript>, String>;

    /// Create the script, or replace the description, schema and body of the
    /// one with the same name.
    async fn save(&self, script: SavedScript) -> Result<SavedScript, String>;
}

/// Tool context extension carrying the bound store.
#[derive(Clone)]
pub struct SavedScripts {
    pub store: Arc<dyn SavedScriptStore>,
    /// Whether this turn may save scripts.
    pub can_save: bool,
}

impl std::fmt::Debug for SavedScripts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedScripts")
            .field("can_save", &self.can_save)
            .finish_non_exhaustive()
    }
}

/// Message metadata key that marks a user message as a script run (D9): the
/// turn it starts runs the named saved script with no model call. Reserved:
/// client-supplied metadata loses it (`strip_reserved_message_metadata`), so
/// only the platform, for a trigger that targets a script, can set it.
pub const SCRIPT_RUN_METADATA_KEY: &str = "everruns_script_run";

/// A saved script a trigger runs instead of sending its message to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScriptRun {
    /// The saved script's name.
    pub script: String,
    /// The script's input object, passed on its stdin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub input: Option<Value>,
    /// When the run fails or stops, hand the result to the agent's model,
    /// which then answers in the same turn. Off: the run only records it.
    #[serde(default)]
    pub wake_agent_on_failure: bool,
}

impl ScriptRun {
    /// The script run a message's metadata asks for, if any.
    pub fn from_metadata(
        metadata: Option<&std::collections::HashMap<String, Value>>,
    ) -> Option<Self> {
        serde_json::from_value(metadata?.get(SCRIPT_RUN_METADATA_KEY)?.clone()).ok()
    }

    /// The `bash` command that runs the script with its input on stdin.
    ///
    /// The input travels in a quoted heredoc, so the shell expands nothing in
    /// it; a delimiter that the JSON could contain is impossible because
    /// serialized JSON never holds a raw newline.
    pub fn command(&self) -> String {
        let input = self
            .input
            .clone()
            .unwrap_or_else(|| Value::Object(Default::default()));
        format!(
            "tools scripts {} <<'__EVERRUNS_SCRIPT_RUN__'\n{}\n__EVERRUNS_SCRIPT_RUN__",
            self.script, input
        )
    }
}

/// Whether `name` is a valid script name.
pub fn is_valid_script_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 64
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowercase_words() {
        for good in ["a", "triage-prs", "daily_report2"] {
            assert!(is_valid_script_name(good), "{good}");
        }
        for bad in ["", "1abc", "Triage", "has space", "-x", &"a".repeat(65)] {
            assert!(!is_valid_script_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_script_run_reads_from_metadata_and_builds_its_command() {
        let run = ScriptRun {
            script: "triage-prs".into(),
            input: Some(serde_json::json!({"repo": "a'b"})),
            wake_agent_on_failure: false,
        };
        let metadata = std::collections::HashMap::from([(
            SCRIPT_RUN_METADATA_KEY.to_string(),
            serde_json::to_value(&run).unwrap(),
        )]);
        assert_eq!(ScriptRun::from_metadata(Some(&metadata)), Some(run.clone()));
        assert_eq!(ScriptRun::from_metadata(None), None);
        assert_eq!(
            run.command(),
            "tools scripts triage-prs <<'__EVERRUNS_SCRIPT_RUN__'\n{\"repo\":\"a'b\"}\n__EVERRUNS_SCRIPT_RUN__"
        );
    }
}
