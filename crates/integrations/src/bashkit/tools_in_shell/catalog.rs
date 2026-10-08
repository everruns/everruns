//! The tools a `tools` command can reach, how they are spelled, and the help
//! and search text that describes them.
//!
//! An MCP tool `mcp_<server>__<tool>` is `tools <server> <tool>`; every other
//! tool is a top-level command, `tools <tool>`. Underscores are typed as
//! hyphens, and either spelling resolves. The catalog is rebuilt from the
//! session's tool registry on every shell call, so a server added mid-session
//! appears on the next one.

use std::collections::BTreeMap;
use std::sync::Arc;

use everruns_contracts::runtime::mcp_server::parse_mcp_tool_name;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::Tool;
use serde_json::Value;

use super::goes_behind_tools;

/// One tool reachable from the shell.
#[derive(Clone)]
pub(crate) struct Entry {
    /// Registry name, the identity the call runs under.
    pub tool_name: String,
    /// MCP server the tool belongs to, as typed (`github`), or `None` for a
    /// top-level command.
    pub source: Option<String>,
    /// The command word as typed (`list-pulls`).
    pub command: String,
    pub tool: Arc<dyn Tool>,
}

impl Entry {
    /// The line that runs this tool, without input.
    pub fn command_line(&self) -> String {
        match &self.source {
            Some(source) => format!("tools {source} {}", self.command),
            None => format!("tools {}", self.command),
        }
    }
}

/// Every tool the shell may call, in a stable order.
pub(crate) struct Catalog {
    entries: Vec<Entry>,
}

/// How a typed word compares to a name: case and `-`/`_` do not matter.
pub(crate) fn normalize(word: &str) -> String {
    word.to_ascii_lowercase().replace('_', "-")
}

fn spell(name: &str) -> String {
    name.replace('_', "-")
}

impl Catalog {
    /// Eligible registry tools. The marker tool and anything that must stay a
    /// direct call are left out, using the same predicate that hides tools from
    /// the model, so a hidden tool is always reachable here.
    pub fn from_context(context: &ToolContext) -> Self {
        let mut entries = Vec::new();
        if let Some(registry) = context.tool_registry.as_ref() {
            for name in registry.tool_names() {
                let Some(tool) = registry.get(name) else {
                    continue;
                };
                if !goes_behind_tools(
                    name,
                    false,
                    &tool.policy(),
                    &tool.deferrable_policy(),
                    &tool.hints(),
                ) {
                    continue;
                }
                let (source, command) = match parse_mcp_tool_name(name) {
                    Some((server, tool_name)) => (Some(spell(&server)), spell(&tool_name)),
                    None => (None, spell(name)),
                };
                entries.push(Entry {
                    tool_name: name.to_string(),
                    source,
                    command,
                    tool: tool.clone(),
                });
            }
        }
        Self::from_entries(entries)
    }

    pub(crate) fn from_entries(mut entries: Vec<Entry>) -> Self {
        entries.sort_by(|a, b| {
            (a.source.as_deref().unwrap_or(""), &a.command)
                .cmp(&(b.source.as_deref().unwrap_or(""), &b.command))
        });
        Self { entries }
    }

    /// Whether `word` names an MCP server in the catalog.
    pub fn is_source(&self, word: &str) -> bool {
        let word = normalize(word);
        self.entries
            .iter()
            .any(|e| e.source.as_deref().is_some_and(|s| normalize(s) == word))
    }

    /// The top-level tool named `word`.
    pub fn top_level(&self, word: &str) -> Option<&Entry> {
        let word = normalize(word);
        self.entries
            .iter()
            .find(|e| e.source.is_none() && normalize(&e.command) == word)
    }

    /// The tool `command` on server `source`.
    pub fn in_source(&self, source: &str, command: &str) -> Option<&Entry> {
        let (source, command) = (normalize(source), normalize(command));
        self.entries.iter().find(|e| {
            e.source.as_deref().is_some_and(|s| normalize(s) == source)
                && normalize(&e.command) == command
        })
    }

    fn sources(&self) -> BTreeMap<&str, usize> {
        let mut out = BTreeMap::new();
        for entry in &self.entries {
            if let Some(source) = &entry.source {
                *out.entry(source.as_str()).or_insert(0) += 1;
            }
        }
        out
    }

    /// `tools --help`.
    pub fn root_help(&self) -> String {
        let mut out = String::from(
            "tools: call the agent's tools from the shell. Input is one JSON object; \
             output is JSON on stdout.\n\n\
             Usage:\n  tools <server> <tool> '{\"key\":\"value\"}'\n  \
             tools <tool> '{...}'\n  jq -n '{key: 1}' | tools <tool>\n  \
             tools <tool> key=value --other-key value\n  \
             tools search <words>\n  tools <server> --help\n  tools <tool> --help\n",
        );
        let sources = self.sources();
        if !sources.is_empty() {
            out.push_str("\nServers:\n");
            for (source, count) in sources {
                let noun = if count == 1 { "tool" } else { "tools" };
                out.push_str(&format!("  {source}  ({count} {noun})\n"));
            }
        }
        let top: Vec<&Entry> = self.entries.iter().filter(|e| e.source.is_none()).collect();
        if !top.is_empty() {
            out.push_str("\nTools:\n");
            for entry in top {
                out.push_str(&format!(
                    "  {}  {}\n",
                    entry.command,
                    first_sentence(entry.tool.description())
                ));
            }
        }
        if self.entries.is_empty() {
            out.push_str("\nNo tools are available from the shell in this session.\n");
        }
        out
    }

    /// `tools <server> --help`.
    pub fn source_help(&self, source: &str) -> String {
        let wanted = normalize(source);
        let mut out = String::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| e.source.as_deref().is_some_and(|s| normalize(s) == wanted))
        {
            if out.is_empty() {
                out.push_str(&format!(
                    "Tools on {}. Run `tools {} <tool> --help` for one tool's input.\n\n",
                    entry.source.as_deref().unwrap_or(source),
                    entry.source.as_deref().unwrap_or(source),
                ));
            }
            out.push_str(&format!(
                "  {}  {}\n",
                entry.command,
                first_sentence(entry.tool.description())
            ));
        }
        out
    }

    /// Ranked matches for `tools search`, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Entry> {
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_ascii_lowercase())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, &Entry)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let name = format!(
                    "{} {}",
                    entry.source.as_deref().unwrap_or(""),
                    entry.command
                )
                .to_ascii_lowercase();
                let description = entry.tool.description().to_ascii_lowercase();
                let score: usize = terms
                    .iter()
                    .map(|term| {
                        // A name hit says far more about intent than an
                        // incidental word in a description.
                        let in_name = if name.contains(term.as_str()) { 3 } else { 0 };
                        let in_description = usize::from(description.contains(term.as_str()));
                        in_name + in_description
                    })
                    .sum();
                (score > 0).then_some((score, entry))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.command.cmp(&b.1.command)));
        scored.into_iter().take(limit).map(|(_, e)| e).collect()
    }

    /// One line per source and top-level tool, for the `bash` tool description.
    pub fn summary_line<'a>(
        entries: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    ) -> String {
        let mut sources: BTreeMap<String, usize> = BTreeMap::new();
        let mut top = Vec::new();
        for (name, server) in entries {
            match server {
                Some(server) => *sources.entry(spell(server)).or_insert(0) += 1,
                None => top.push(spell(name)),
            }
        }
        let mut parts: Vec<String> = sources
            .into_iter()
            .map(|(source, count)| format!("{source} ({count})"))
            .collect();
        top.sort();
        parts.extend(top);
        parts.join(", ")
    }
}

/// `tools <...> <tool> --help`: the signature, description and input schema.
pub(crate) fn tool_help(entry: &Entry) -> String {
    let schema = strip_human_intent(entry.tool.parameters_schema());
    format!(
        "{} '{}'\n\n{}\n\nInput schema:\n{}\n",
        entry.command_line(),
        example_input(&schema),
        entry.tool.description().trim(),
        serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string()),
    )
}

/// `{"repo":"<string>","state?":"<string>"}`: required keys first.
fn example_input(schema: &Value) -> String {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "{}".to_string();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut parts = Vec::new();
    for key in &required {
        if let Some(property) = properties.get(*key) {
            parts.push(format!("\"{key}\":<{}>", json_type(property)));
        }
    }
    for (key, property) in properties {
        if !required.contains(&key.as_str()) {
            parts.push(format!("\"{key}?\":<{}>", json_type(property)));
        }
    }
    format!("{{{}}}", parts.join(","))
}

fn json_type(property: &Value) -> String {
    match property.get("type") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("|"),
        _ if property.get("enum").is_some() => "enum".to_string(),
        _ => "any".to_string(),
    }
}

/// The synthetic narration argument is the model's to fill on a direct call;
/// nothing in a script should be asked for it.
pub(crate) fn strip_human_intent(mut schema: Value) -> Value {
    const HUMAN_INTENT: &str = everruns_contracts::tool_types::HUMAN_INTENT_ARGUMENT;
    if let Some(object) = schema.as_object_mut() {
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove(HUMAN_INTENT);
        }
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|v| v.as_str() != Some(HUMAN_INTENT));
        }
    }
    schema
}

/// First sentence (up to `.` or newline), trimmed and length-capped.
pub(crate) fn first_sentence(description: &str) -> String {
    let trimmed = description.trim();
    let end = trimmed
        .find(['.', '\n'])
        .map(|i| i + 1)
        .unwrap_or(trimmed.len());
    let mut s = trimmed[..end].trim().to_string();
    const MAX: usize = 120;
    if s.len() > MAX {
        let boundary = s
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= MAX)
            .last()
            .unwrap_or(0);
        s.truncate(boundary);
        s.push('…');
    }
    s
}
