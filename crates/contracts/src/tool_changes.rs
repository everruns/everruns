//! Recorded tool changes: how the tool list a model sees evolves within a
//! session without rewriting the cached request prefix.
//!
//! The first request of a session (or of a compaction or model epoch) fixes
//! a baseline; every later difference between what the model has been told it can call and
//! what it can call now is one [`ToolChanges`] entry, placed in the
//! conversation where it happened. Replaying the baseline and the entries in
//! order always yields the tools the conversation has seen, so the transcript,
//! not process memory, is the record.
//!
//! Removal is soft: a removed tool stays in the declared list (the cached
//! prefix keeps it) and is only announced as gone.

// Design of record: knowledge/execution/cache-stable-tool-changes.md.
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};

/// Metadata key of the completed assistant message that stores the entry
/// recorded before its generation.
pub const METADATA_KEY: &str = "tool_changes";

/// One recorded change to the tools the model may call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolChanges {
    /// Starts a new epoch: the full tool list sent as the request's `tools`.
    /// Present on the first entry of a session, after compaction and after a
    /// model switch; entries without it apply on top of the latest baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Vec<ToolDefinition>>,
    /// Tools that become callable: new names, or names announced as removed
    /// earlier and offered again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<ToolDefinition>,
    /// Full definitions that replace a declared definition of the same name
    /// (a deferred stub becoming its full schema is a redefinition).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redefined: Vec<ToolDefinition>,
    /// Names that are no longer callable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Prompt text that comes with the change, such as a capability's how-to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ToolChanges {
    /// An entry that starts a new epoch with `tools` as the baseline.
    pub fn baseline(tools: Vec<ToolDefinition>) -> Self {
        Self {
            baseline: Some(tools),
            ..Self::default()
        }
    }

    /// True when the entry changes nothing on top of its baseline.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.redefined.is_empty()
            && self.removed.is_empty()
            && self.note.is_none()
    }

    /// Names this entry touches, for logs and traces.
    pub fn changed_names(&self) -> impl Iterator<Item = &str> {
        self.added
            .iter()
            .chain(&self.redefined)
            .map(ToolDefinition::name)
            .chain(self.removed.iter().map(String::as_str))
    }
}

/// A tool the conversation has declared, and whether it was later removed.
#[derive(Debug, Clone)]
pub struct DeclaredTool {
    pub definition: ToolDefinition,
    pub removed: bool,
}

/// The tools a conversation has declared so far: a baseline plus every
/// recorded change, in declaration order.
#[derive(Debug, Clone, Default)]
pub struct DeclaredTools {
    tools: Vec<DeclaredTool>,
}

impl DeclaredTools {
    /// Start from a baseline.
    pub fn new(baseline: Vec<ToolDefinition>) -> Self {
        Self {
            tools: baseline
                .into_iter()
                .map(|definition| DeclaredTool {
                    definition,
                    removed: false,
                })
                .collect(),
        }
    }

    /// Every declared tool, removed ones included.
    pub fn tools(&self) -> &[DeclaredTool] {
        &self.tools
    }

    /// The tools that are callable now, in declaration order.
    pub fn callable(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools
            .iter()
            .filter(|tool| !tool.removed)
            .map(|tool| &tool.definition)
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.tools
            .iter()
            .position(|tool| tool.definition.name() == name)
    }

    /// Apply one recorded entry. A baseline restarts the declaration.
    pub fn apply(&mut self, changes: &ToolChanges) {
        if let Some(baseline) = &changes.baseline {
            *self = Self::new(baseline.clone());
        }
        for definition in changes.added.iter().chain(&changes.redefined) {
            match self.position(definition.name()) {
                Some(index) => {
                    self.tools[index] = DeclaredTool {
                        definition: definition.clone(),
                        removed: false,
                    }
                }
                None => self.tools.push(DeclaredTool {
                    definition: definition.clone(),
                    removed: false,
                }),
            }
        }
        for name in &changes.removed {
            if let Some(index) = self.position(name) {
                self.tools[index].removed = true;
            }
        }
    }

    /// The entry that turns this declaration into `wanted`: wanted tools not
    /// declared (or declared and removed) are added, declared tools whose
    /// model-visible definition differs are redefined, and callable tools that
    /// are no longer wanted are removed. Empty when nothing differs.
    pub fn diff(&self, wanted: &[ToolDefinition]) -> ToolChanges {
        let mut changes = ToolChanges::default();
        for definition in wanted {
            match self.position(definition.name()).map(|i| &self.tools[i]) {
                None => changes.added.push(definition.clone()),
                Some(tool) if tool.removed => changes.added.push(definition.clone()),
                Some(tool) if !same_for_model(&tool.definition, definition) => {
                    changes.redefined.push(definition.clone())
                }
                Some(_) => {}
            }
        }
        for tool in self.tools.iter().filter(|tool| !tool.removed) {
            let name = tool.definition.name();
            if !wanted.iter().any(|definition| definition.name() == name) {
                changes.removed.push(name.to_string());
            }
        }
        changes
    }
}

/// Whether two definitions of one name look the same to a model. Only what a
/// driver sends counts: approval policy, display names and hints never reach
/// the provider, so changing them is not a tool change.
pub fn same_for_model(a: &ToolDefinition, b: &ToolDefinition) -> bool {
    a.name() == b.name()
        && a.description() == b.description()
        && a.parameters() == b.parameters()
        && a.deferrable() == b.deferrable()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::ToolPolicy;
    use serde_json::json;

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::function(name, description, json!({"type": "object"}))
    }

    fn names(tools: &[ToolDefinition]) -> Vec<&str> {
        tools.iter().map(ToolDefinition::name).collect()
    }

    #[test]
    fn same_tools_need_no_change() {
        let declared = DeclaredTools::new(vec![tool("a", "A"), tool("b", "B")]);
        assert!(declared.diff(&[tool("b", "B"), tool("a", "A")]).is_empty());
    }

    #[test]
    fn diff_finds_added_redefined_and_removed() {
        let declared = DeclaredTools::new(vec![tool("a", "A"), tool("b", "B"), tool("c", "C")]);
        let changes = declared.diff(&[tool("a", "A"), tool("b", "B, now longer"), tool("d", "D")]);
        assert_eq!(names(&changes.added), ["d"]);
        assert_eq!(names(&changes.redefined), ["b"]);
        assert_eq!(changes.removed, ["c"]);
        assert!(changes.baseline.is_none());
    }

    #[test]
    fn replaying_the_diff_reaches_the_wanted_set() {
        let mut declared = DeclaredTools::new(vec![tool("a", "A"), tool("c", "C")]);
        let wanted = vec![tool("a", "A2"), tool("b", "B")];
        declared.apply(&declared.diff(&wanted));
        assert!(declared.diff(&wanted).is_empty());
        // Soft removal: `c` stays declared, announced as removed.
        let all: Vec<_> = declared
            .tools()
            .iter()
            .map(|tool| (tool.definition.name(), tool.removed))
            .collect();
        assert_eq!(all, [("a", false), ("c", true), ("b", false)]);
    }

    #[test]
    fn a_removed_tool_offered_again_is_an_addition() {
        let mut declared = DeclaredTools::new(vec![tool("a", "A"), tool("b", "B")]);
        declared.apply(&declared.diff(&[tool("a", "A")]));
        let changes = declared.diff(&[tool("a", "A"), tool("b", "B")]);
        assert_eq!(names(&changes.added), ["b"]);
        assert!(changes.removed.is_empty());
        declared.apply(&changes);
        assert_eq!(declared.callable().count(), 2);
    }

    #[test]
    fn fields_the_model_never_sees_are_not_a_change() {
        let ToolDefinition::ClientSide(mut quiet) = tool("a", "A") else {
            unreachable!()
        };
        quiet.display_name = Some("Tool A".into());
        let declared = DeclaredTools::new(vec![tool("a", "A")]);
        assert!(
            declared
                .diff(&[ToolDefinition::ClientSide(quiet)])
                .is_empty()
        );

        let builtin = |policy| {
            ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
                name: "b".into(),
                display_name: None,
                description: "B".into(),
                parameters: json!({}),
                policy,
                category: None,
                deferrable: Default::default(),
                hints: Default::default(),
                full_parameters: None,
            })
        };
        let declared = DeclaredTools::new(vec![builtin(ToolPolicy::Auto)]);
        assert!(
            declared
                .diff(&[builtin(ToolPolicy::RequiresApproval)])
                .is_empty()
        );
    }

    #[test]
    fn a_baseline_restarts_the_declaration() {
        let mut declared = DeclaredTools::new(vec![tool("a", "A")]);
        declared.apply(&ToolChanges::baseline(vec![tool("z", "Z")]));
        assert_eq!(
            declared
                .callable()
                .map(ToolDefinition::name)
                .collect::<Vec<_>>(),
            ["z"]
        );
    }

    #[test]
    fn entries_round_trip_and_omit_empty_parts() {
        let changes = ToolChanges {
            removed: vec!["c".into()],
            ..ToolChanges::default()
        };
        let value = serde_json::to_value(&changes).unwrap();
        assert_eq!(value, json!({"removed": ["c"]}));
        let back = serde_json::from_value::<ToolChanges>(value).unwrap();
        assert_eq!(back.removed, changes.removed);
        assert!(back.baseline.is_none() && back.added.is_empty());
    }
}
