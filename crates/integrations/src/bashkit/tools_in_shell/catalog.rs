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

use everruns_contracts::runtime::mcp_deferred::deferred_mcp_server_prefix;
use everruns_contracts::runtime::mcp_server::parse_mcp_tool_name;
use everruns_contracts::runtime::tool_catalog::{ToolSearchIndex, ToolSearchSource};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::Tool;
use everruns_contracts::tool_types::{ToolDefinition, ToolHints};
use serde_json::Value;
use std::collections::HashSet;

use everruns_contracts::runtime::tool_surface::{ToolSurface, ToolSurfacePlan};

/// Bound aggregate catalog work for one shell call to the same session-wide
/// discovery limits used by the host.
const MAX_CATALOG_TOOLS: usize = 10_000;
const MAX_CATALOG_BYTES: usize = 32 * 1024 * 1024;
const MAX_PENDING_LOADS_PER_SEARCH: usize = 8;
const MAX_HELP_ENTRIES: usize = 64;
const MAX_HELP_BYTES: usize = 12 * 1024;
const MAX_HELP_LINE_BYTES: usize = 256;
const HELP_OMISSION_RESERVE_BYTES: usize = 160;

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
    /// Registry owner context survives a deferred-load index rebuild.
    pub hints: ToolHints,
    /// Deferred children are authorized through their listed parent placeholder.
    pub scope_parent: Option<String>,
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

/// A deferred MCP server ("load on demand") whose tools are not listed yet.
/// The first command or search that names it loads it.
#[derive(Clone)]
pub(crate) struct Pending {
    /// Tool prefix, as in `mcp_<prefix>__<tool>`.
    pub prefix: String,
    /// The server as typed (`github`).
    pub source: String,
    /// The placeholder tool, whose description says what the server is for.
    pub placeholder: Arc<dyn Tool>,
    pub hints: ToolHints,
}

/// Every tool the shell may call, in a stable order, plus the deferred
/// servers not loaded yet.
#[derive(Clone)]
pub(crate) struct Catalog {
    entries: Vec<Entry>,
    pending: Vec<Pending>,
    search_index: Option<Arc<ToolSearchIndex>>,
    searchable_names: HashSet<String>,
    surface_plan: ToolSurfacePlan,
    visible_tool_names: Option<Arc<HashSet<String>>>,
}

/// How a typed word compares to a name: case and `-`/`_` do not matter.
pub(crate) fn normalize(word: &str) -> String {
    word.to_ascii_lowercase().replace('_', "-")
}

fn spell(name: &str) -> String {
    name.replace('_', "-")
}

impl Catalog {
    /// Eligible registry tools admitted by the resolved surface plan. A missing
    /// route is fail-closed, including for MCP tools loaded after assembly.
    pub fn from_context(context: &ToolContext) -> Self {
        let mut entries = Vec::new();
        let mut pending = Vec::new();
        let mut searchable_names = HashSet::new();
        let search_index = context
            .tool_registry
            .as_ref()
            .map(|registry| registry.search_index());
        let surface_plan = context.tool_surface_plan.clone();
        if let Some(registry) = context.tool_registry.as_ref() {
            for name in registry.tool_names() {
                let Some(tool) = registry.get(name) else {
                    continue;
                };
                let Some(definition) = registry.tool_definition(name) else {
                    continue;
                };
                if context
                    .visible_tool_names
                    .as_ref()
                    .is_some_and(|visible| !visible.contains(name))
                {
                    continue;
                }
                if !context
                    .tool_surface_plan
                    .route_for(name)
                    .is_some_and(|route| {
                        matches!(route.surface, ToolSurface::Cli | ToolSurface::Both)
                    })
                {
                    continue;
                }
                if let Some(prefix) = deferred_mcp_server_prefix(name) {
                    searchable_names.insert(name.to_string());
                    pending.push(Pending {
                        prefix: prefix.to_string(),
                        source: spell(prefix),
                        placeholder: tool.clone(),
                        hints: definition.hints().clone(),
                    });
                    continue;
                }
                searchable_names.insert(name.to_string());
                let (source, command) = match parse_mcp_tool_name(name) {
                    Some((server, tool_name)) => (Some(spell(&server)), spell(&tool_name)),
                    None => (None, spell(name)),
                };
                entries.push(Entry {
                    tool_name: name.to_string(),
                    source,
                    command,
                    tool: tool.clone(),
                    hints: definition.hints().clone(),
                    scope_parent: None,
                });
            }
        }
        pending.sort_by(|a, b| a.source.cmp(&b.source));
        let mut catalog = Self {
            entries,
            pending,
            search_index,
            searchable_names,
            surface_plan,
            visible_tool_names: context.visible_tool_names.clone(),
        };
        catalog.sort();
        catalog
    }

    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            (a.source.as_deref().unwrap_or(""), &a.command)
                .cmp(&(b.source.as_deref().unwrap_or(""), &b.command))
        });
    }

    /// Whether `word` names an MCP server in the catalog, loaded or not.
    pub fn is_source(&self, word: &str) -> bool {
        let word = normalize(word);
        self.entries
            .iter()
            .any(|e| e.source.as_deref().is_some_and(|s| normalize(s) == word))
            || self.pending.iter().any(|p| normalize(&p.source) == word)
    }

    /// The deferred server `word` names, if it is not loaded yet.
    pub fn pending_source(&self, word: &str) -> Option<Pending> {
        let word = normalize(word);
        self.pending
            .iter()
            .find(|p| normalize(&p.source) == word)
            .cloned()
    }

    /// Deferred servers whose name or description matches a search term.
    pub fn pending_matching(&self, query: &str) -> Vec<Pending> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let Some(index) = &self.search_index else {
            return Vec::new();
        };
        let by_tool_name: BTreeMap<_, _> = self
            .pending
            .iter()
            .map(|pending| (format!("mcp_{}", pending.prefix), pending))
            .collect();
        index
            .search(query, MAX_PENDING_LOADS_PER_SEARCH, |name| {
                by_tool_name.contains_key(name)
            })
            .into_iter()
            .filter_map(|hit| {
                by_tool_name
                    .get(&hit.name)
                    .map(|pending| (*pending).clone())
            })
            .collect()
    }

    /// A deferred server's tools arrived: they join the catalog and the server
    /// is no longer pending.
    pub fn add_loaded(&mut self, prefix: &str, tools: Vec<Arc<dyn Tool>>) -> Result<(), String> {
        let Some(parent) = self.pending.iter().find(|p| p.prefix == prefix) else {
            return Err(format!(
                "MCP server {prefix} is no longer an authorized pending server"
            ));
        };
        let parent_name = parent.placeholder.name().to_string();
        if self
            .visible_tool_names
            .as_ref()
            .is_some_and(|visible| !visible.contains(&parent_name))
        {
            return Err(format!(
                "MCP server {prefix} is outside the current tool scope"
            ));
        }

        let mut additions = Vec::new();
        let mut candidate_plan = self.surface_plan.clone();
        for tool in tools {
            let definition = tool.to_definition();
            let Some((server, tool_name)) = parse_mcp_tool_name(tool.name()) else {
                return Err(format!(
                    "MCP server {prefix} returned a tool outside its namespace"
                ));
            };
            if server != prefix {
                return Err(format!("MCP server {prefix} returned a tool from {server}"));
            }
            if candidate_plan.route_for(definition.name()).is_none() {
                candidate_plan
                    .extend_definitions(std::slice::from_ref(&definition), |definition| {
                        super::definition_goes_behind_tools(definition)
                    });
            }
            if !candidate_plan
                .route_for(definition.name())
                .is_some_and(|route| matches!(route.surface, ToolSurface::Cli | ToolSurface::Both))
            {
                continue;
            }
            additions.push(Entry {
                tool_name: tool.name().to_string(),
                source: Some(spell(&server)),
                command: spell(&tool_name),
                tool,
                hints: definition.hints().clone(),
                scope_parent: Some(parent_name.clone()),
            });
        }

        let other_pending = self.pending.iter().filter(|p| p.prefix != prefix);
        let count = self.entries.len() + other_pending.clone().count() + additions.len();
        let bytes = self
            .entries
            .iter()
            .map(entry_bytes)
            .chain(other_pending.map(pending_bytes))
            .chain(additions.iter().map(entry_bytes))
            .try_fold(0usize, usize::checked_add)
            .unwrap_or(usize::MAX);
        if count > MAX_CATALOG_TOOLS || bytes > MAX_CATALOG_BYTES {
            return Err(format!(
                "loading MCP server {prefix} would exceed the shell catalog limit ({MAX_CATALOG_TOOLS} tools or {} bytes)",
                MAX_CATALOG_BYTES
            ));
        }

        self.surface_plan = candidate_plan;
        self.pending.retain(|pending| pending.prefix != prefix);
        self.searchable_names.remove(&parent_name);
        for entry in additions {
            self.searchable_names.insert(entry.tool_name.clone());
            self.entries.push(entry);
        }
        self.sort();
        let definitions: Vec<_> = self
            .entries
            .iter()
            .map(|entry| with_hints(entry.tool.to_definition(), &entry.hints))
            .chain(
                self.pending
                    .iter()
                    .map(|pending| with_hints(pending.placeholder.to_definition(), &pending.hints)),
            )
            .collect();
        if !definitions.is_empty() {
            self.search_index = Some(Arc::new(ToolSearchIndex::new(&definitions)));
        } else {
            self.search_index = None;
        }
        Ok(())
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
             Discovery: search by intent first with `tools search <words>`; it returns \
             runnable command lines. There is no `tools list` command. Use help to browse a small \
             catalog or inspect one tool.\n\n\
             Usage:\n  tools <server> <tool> '{\"key\":\"value\"}'\n  \
             tools <tool> '{...}'\n  jq -n '{key: 1}' | tools <tool>\n  \
             tools <tool> key=value --other-key value\n  \
             tools search <words>\n  tools <server> --help\n  tools <tool> --help\n  \
             tools plan < script.sh   (list a script's calls and their risk; runs nothing)\n",
        );
        let mut remaining_entries = MAX_HELP_ENTRIES;
        let mut remaining_bytes = MAX_HELP_BYTES
            .saturating_sub(out.len())
            .saturating_sub(HELP_OMISSION_RESERVE_BYTES);
        let top: Vec<&Entry> = self.entries.iter().filter(|e| e.source.is_none()).collect();
        let sources = self.sources();
        let mut omitted_servers = 0;
        let mut omitted_tools = 0;
        if !sources.is_empty() || !self.pending.is_empty() {
            out.push_str("\nServers:\n");
            remaining_bytes = remaining_bytes.saturating_sub("\nServers:\n".len());
            let source_count = sources.len() + self.pending.len();
            let mut rows = Vec::with_capacity(source_count);
            for (source, count) in sources {
                let noun = if count == 1 { "tool" } else { "tools" };
                rows.push(format!("  {source}  ({count} {noun})"));
            }
            for pending in &self.pending {
                rows.push(format!(
                    "  {}  (not loaded; `tools {} --help` loads it)",
                    pending.source, pending.source
                ));
            }
            omitted_servers = append_help_rows(
                &mut out,
                &rows,
                &mut remaining_entries,
                &mut remaining_bytes,
            );
        }
        if !top.is_empty() {
            out.push_str("\nTools:\n");
            remaining_bytes = remaining_bytes.saturating_sub("\nTools:\n".len());
            let rows: Vec<_> = top
                .iter()
                .map(|entry| {
                    format!(
                        "  {}  {}",
                        entry.command,
                        first_sentence(entry.tool.description())
                    )
                })
                .collect();
            omitted_tools = append_help_rows(
                &mut out,
                &rows,
                &mut remaining_entries,
                &mut remaining_bytes,
            );
        }
        if omitted_servers > 0 || omitted_tools > 0 {
            let note = omission_note(omitted_servers, omitted_tools);
            debug_assert!(out.len() + note.len() <= MAX_HELP_BYTES);
            out.push_str(&note);
        }
        if self.entries.is_empty() && self.pending.is_empty() {
            out.push_str("\nNo tools are available from the shell in this session.\n");
        }
        out
    }

    /// `tools <server> --help`.
    pub fn source_help(&self, source: &str) -> String {
        let wanted = normalize(source);
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.source.as_deref().is_some_and(|s| normalize(s) == wanted))
            .collect();
        if entries.is_empty() {
            return String::new();
        }

        let source = entries[0].source.as_deref().unwrap_or(source);
        let header = bounded_line(&format!(
            "Tools on {source}. Run `tools {source} <tool> --help` for one tool's input."
        ));
        let mut out = format!("{header}\n\n");
        // Collecting complete rows keeps truncation on line boundaries and
        // makes the omission count accurate even for large MCP servers.
        let rows: Vec<_> = entries
            .iter()
            .map(|entry| {
                format!(
                    "  {}  {}",
                    entry.command,
                    first_sentence(entry.tool.description())
                )
            })
            .collect();
        let mut count = MAX_HELP_ENTRIES;
        let mut bytes = MAX_HELP_BYTES
            .saturating_sub(out.len())
            .saturating_sub(HELP_OMISSION_RESERVE_BYTES);
        let omitted = append_help_rows(&mut out, &rows, &mut count, &mut bytes);
        if omitted > 0 {
            let note = format!(
                "  ... {omitted} more omitted from this server; use `tools search <words>` to find operations.\n"
            );
            debug_assert!(out.len() + note.len() <= MAX_HELP_BYTES);
            out.push_str(&note);
        }
        out
    }

    /// Ranked matches for `tools search`, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Entry> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let Some(index) = &self.search_index else {
            return Vec::new();
        };
        let by_name: BTreeMap<_, _> = self
            .entries
            .iter()
            .map(|entry| (entry.tool_name.as_str(), entry))
            .collect();
        index
            .search_operations(query, limit, |name| self.searchable_names.contains(name))
            .into_iter()
            .filter_map(|hit| by_name.get(hit.name.as_str()).copied())
            .collect()
    }

    /// Authorized sources whose metadata matched.
    /// These are browse suggestions, never operation matches.
    pub fn source_matches(&self, query: &str, limit: usize) -> Vec<ToolSearchSource> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let Some(index) = &self.search_index else {
            return Vec::new();
        };
        index.search_sources(query, limit, |name| self.searchable_names.contains(name))
    }
}

fn with_hints(mut definition: ToolDefinition, hints: &ToolHints) -> ToolDefinition {
    match &mut definition {
        ToolDefinition::Builtin(tool) => tool.hints = hints.clone(),
        ToolDefinition::ClientSide(tool) => tool.hints = hints.clone(),
    }
    definition
}

/// Append complete, byte-bounded rows. Every row is shortened at a UTF-8
/// boundary before accounting, so help stays valid text and names remain
/// recognizable without dumping a whole large catalog into context.
fn append_help_rows(
    out: &mut String,
    rows: &[String],
    remaining_entries: &mut usize,
    remaining_bytes: &mut usize,
) -> usize {
    let mut shown = 0;
    for row in rows {
        if *remaining_entries == 0 {
            break;
        }
        let row = bounded_line(row);
        let cost = row.len() + 1;
        if cost > *remaining_bytes {
            break;
        }
        out.push_str(&row);
        out.push('\n');
        *remaining_bytes -= cost;
        *remaining_entries -= 1;
        shown += 1;
    }
    rows.len() - shown
}

fn omission_note(omitted_servers: usize, omitted_tools: usize) -> String {
    let server_label = if omitted_servers == 1 {
        "server entry"
    } else {
        "server entries"
    };
    let tool_label = if omitted_tools == 1 {
        "top-level tool"
    } else {
        "top-level tools"
    };
    let details = match (omitted_servers > 0, omitted_tools > 0) {
        (true, true) => {
            format!("{omitted_servers} {server_label} and {omitted_tools} {tool_label}")
        }
        (true, false) => format!("{omitted_servers} {server_label}"),
        (false, true) => format!("{omitted_tools} {tool_label}"),
        (false, false) => return String::new(),
    };
    format!("  ... {details} omitted; use `tools search <words>` to find operations.\n")
}

pub(super) fn bounded_line(line: &str) -> String {
    if line.len() <= MAX_HELP_LINE_BYTES {
        return line.to_string();
    }
    let limit = MAX_HELP_LINE_BYTES - '…'.len_utf8();
    let boundary = line
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= limit)
        .last()
        .unwrap_or(0);
    format!("{}…", &line[..boundary])
}

fn entry_bytes(entry: &Entry) -> usize {
    serde_json::to_vec(&entry.tool.to_definition())
        .map(|definition| definition.len())
        .unwrap_or(usize::MAX)
}

fn pending_bytes(pending: &Pending) -> usize {
    serde_json::to_vec(&pending.placeholder.to_definition())
        .map(|definition| definition.len())
        .unwrap_or(usize::MAX)
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_contracts::runtime::ToolExecutionResult;
    use everruns_contracts::runtime::mcp_deferred::{
        DeferredMcpServerTool, deferred_mcp_server_definition,
    };
    use everruns_contracts::runtime::tool_surface::ToolSurfaceMode;
    use everruns_contracts::tool_types::ToolDefinition;

    fn catalog_for_source(count: usize) -> Catalog {
        let entries: Vec<_> = (0..count)
            .map(|index| {
                let tool_name = format!("mcp_telemetry__event_{index:04}");
                Entry {
                    command: format!("event-{index:04}"),
                    source: Some("telemetry".into()),
                    tool: Arc::new(NamedTool(tool_name.clone())),
                    hints: ToolHints::default(),
                    tool_name,
                    scope_parent: None,
                }
            })
            .collect();
        Catalog {
            searchable_names: entries
                .iter()
                .map(|entry| entry.tool_name.clone())
                .collect(),
            entries,
            pending: Vec::new(),
            search_index: None,
            surface_plan: ToolSurfacePlan::for_mode(ToolSurfaceMode::Cli),
            visible_tool_names: None,
        }
    }

    #[test]
    fn root_help_bounds_a_catalog_with_many_servers() {
        let mut catalog = catalog_for_source(2_000);
        for (index, entry) in catalog.entries.iter_mut().enumerate() {
            entry.source = Some(format!("server-{index:04}{}", "x".repeat(157)));
        }
        for name in ["top_one", "top_two"] {
            catalog.searchable_names.insert(name.to_string());
            catalog.entries.push(Entry {
                tool_name: name.to_string(),
                source: None,
                command: name.replace('_', "-"),
                tool: Arc::new(NamedTool(name.to_string())),
                hints: ToolHints::default(),
                scope_parent: None,
            });
        }
        let output = catalog.root_help();

        assert!(output.len() <= MAX_HELP_BYTES, "{} bytes", output.len());
        assert!(output.contains("omitted; use `tools search"), "{output}");
        assert!(output.contains("tools search <words>"), "{output}");
        assert!(
            output.contains("1 top-level tool omitted"),
            "the final note must account for the omitted top-level section: {output}"
        );
    }

    #[test]
    fn large_server_help_is_bounded_and_points_to_search() {
        let catalog = catalog_for_source(2_000);
        let output = catalog.source_help("telemetry");

        assert!(output.len() <= MAX_HELP_BYTES, "{} bytes", output.len());
        assert!(
            output.contains("event-0000"),
            "first entry should remain visible"
        );
        assert!(
            output.contains("more omitted"),
            "omissions must be explicit"
        );
        assert!(
            output.contains("tools search <words>"),
            "help should teach search"
        );
        assert!(
            !output.contains("event-1999"),
            "the full catalog must not be dumped"
        );
    }

    #[test]
    fn small_server_help_still_lists_every_operation() {
        let catalog = catalog_for_source(2);
        let output = catalog.source_help("telemetry");

        assert!(output.contains("event-0000"), "{output}");
        assert!(output.contains("event-0001"), "{output}");
        assert!(!output.contains("omitted"), "{output}");
        assert!(output.len() <= MAX_HELP_BYTES);
    }

    #[test]
    fn help_line_limit_preserves_unicode_boundaries() {
        let output = bounded_line(&"é".repeat(MAX_HELP_LINE_BYTES));

        assert!(output.len() <= MAX_HELP_LINE_BYTES);
        assert!(output.ends_with('…'));
        assert!(std::str::from_utf8(output.as_bytes()).is_ok());
    }

    struct NamedTool(String);

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            &self.0
        }

        fn description(&self) -> &str {
            "Test tool."
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::Success(serde_json::json!({}))
        }
    }

    #[test]
    fn aggregate_limit_rejects_a_whole_server_without_partial_publish() {
        let ToolDefinition::Builtin(parent_definition) =
            deferred_mcp_server_definition("docs", Some("Documentation"))
        else {
            unreachable!()
        };
        let parent: Arc<dyn Tool> = Arc::new(DeferredMcpServerTool::new(parent_definition));
        let entries = (0..MAX_CATALOG_TOOLS - 1)
            .map(|index| Entry {
                tool_name: format!("existing_{index}"),
                source: None,
                command: format!("existing-{index}"),
                tool: parent.clone(),
                hints: ToolHints::default(),
                scope_parent: None,
            })
            .collect::<Vec<_>>();
        let mut catalog = Catalog {
            searchable_names: entries
                .iter()
                .map(|entry| entry.tool_name.clone())
                .collect(),
            entries,
            pending: vec![Pending {
                prefix: "docs".into(),
                source: "docs".into(),
                placeholder: parent,
                hints: ToolHints::default(),
            }],
            search_index: None,
            surface_plan: ToolSurfacePlan::for_mode(ToolSurfaceMode::Cli),
            visible_tool_names: None,
        };

        let tools: Vec<Arc<dyn Tool>> = ["mcp_docs__one", "mcp_docs__two"]
            .into_iter()
            .map(|name| Arc::new(NamedTool(name.into())) as Arc<dyn Tool>)
            .collect();
        let error = catalog.add_loaded("docs", tools).unwrap_err();

        assert!(error.contains("catalog limit"));
        assert_eq!(catalog.entries.len(), MAX_CATALOG_TOOLS - 1);
        assert_eq!(catalog.pending.len(), 1);
        assert!(!catalog.searchable_names.contains("mcp_docs__one"));
    }
}
