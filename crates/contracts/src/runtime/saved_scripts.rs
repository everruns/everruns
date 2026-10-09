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
}
