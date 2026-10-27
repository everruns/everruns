// Generic (provider-agnostic) Tool Search Capability
//
// Brings deferred tool loading to models that have no native tool_search
// support (Anthropic, Gemini, OpenAI Completions, ...). Unlike
// `openai_tool_search`, which relies on the OpenAI Responses API to hide
// parameter schemas server-side, this capability implements tool search
// entirely client-side and therefore works with any provider.
//
// How it works:
//   1. A `tool_definition_hook` (`DeferSchemaHook`) runs at runtime-agent build
//      time. When the agent carries at least `threshold` tools, it replaces the
//      parameter schema of every deferrable tool with a minimal disclosure stub.
//      The model still sees that the tool exists, while the single capability
//      prompt points it to `tool_search` instead of repeating that instruction
//      in every stub. Tools marked `DeferrablePolicy::Never` (e.g. high-frequency
//      tools) and tools in the capability's `never_defer` allowlist keep full
//      schemas.
//   2. A real `tool_search` tool is added to the registry. When the model calls
//      it, the tool inspects its sibling tools via `ToolContext::tool_registry`
//      (the same mechanism `spawn_background` uses) and returns the full
//      parameter schemas of the tools matching the query. It also records those
//      tools as *revealed* for the calling session (see below).
//   3. A short system-prompt note tells the model to call `tool_search` before
//      using a tool whose parameters it has not loaded yet.
//
// Because the underlying tools stay registered and executable, tool calls and
// results work exactly as before — the only difference is how schemas reach the
// model. No driver or agent-loop changes are required.
//
// Progressive disclosure (session-scoped)
// ---------------------------------------
// Structured tool calling makes the model emit arguments against a tool's
// *registered* schema. If a deferred tool's registered schema stayed the stub
// forever, the model could read the real schema from a `tool_search` *result*
// but still have no registered schema to emit arguments against. To close that
// gap, `tool_search` records its matches as revealed; because the hook re-runs
// when turn context is reassembled, a later pass advertises those tools with
// their full, authoritative schema on the *registered* definition — so the model
// can finally pass arguments. The permissive stub (`additionalProperties: true`)
// remains as a belt-and-suspenders for the first call before a reveal lands.
//
// The record of what was revealed is the conversation itself, not process
// memory: `tool_disclosure` reads the `tool_search` results and the recorded
// tool changes from the transcript, and context assembly hands the result to
// the hook through `SystemPromptContext::tool_disclosure`. Reveals therefore
// survive a worker restart and look the same on every worker, and the
// capability, a process-global singleton, holds no per-session state.
//
// Never-defer allowlist
// ---------------------
// `DeferrablePolicy::Never` lets a tool *owner* opt a tool out of deferral. But
// an embedder that composes tools it does not own (e.g. file/shell tools from
// another crate) cannot change their policy. `ToolSearchCapability::with_never_defer`
// (and a `never_defer` config array) lets such an embedder keep hot-path tools
// fully loaded by name, so the agent is never forced through a `tool_search`
// round-trip before its first read/edit/shell call. Equivalent in effect to
// marking those tools `DeferrablePolicy::Never`, but settable from outside.

use super::{
    Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext, ToolDefinitionHook,
};
use crate::builtins::tool_types::{DeferrablePolicy, ToolDefinition, ToolHints};
use crate::builtins::tools::{Tool, ToolExecutionResult};
use crate::tool_context::ToolContext;
use async_trait::async_trait;
use everruns_contracts::runtime::capabilities::ToolDisclosure;
use everruns_contracts::runtime::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use everruns_contracts::tool_changes::ToolChanges;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for the generic (provider-agnostic) tool search.
pub const TOOL_SEARCH_CAPABILITY_ID: &str = "tool_search";

/// Name of the tool the model calls to load deferred schemas.
pub const TOOL_SEARCH_TOOL_NAME: &str = "tool_search";

/// Maximum number of tools returned (and revealed) by a single `tool_search`
/// call. Kept small: every returned tool is also recorded as *revealed*, which
/// un-defers its full schema for the rest of the session (see "Progressive
/// disclosure" above), so a loose query must not permanently un-defer a large
/// slice of the catalogue.
const MAX_SEARCH_RESULTS: usize = 8;

/// Per-term score for a query term hit in the tool *name* vs its *description*.
/// A name hit is a far stronger signal of intent than an incidental word in a
/// description, so it is weighted higher.
const NAME_TERM_WEIGHT: usize = 3;
const DESC_TERM_WEIGHT: usize = 1;

/// Dominant bonus when the whole query is exactly a tool name. This is the
/// common "load this specific tool" path and that tool
/// must rank first regardless of incidental matches elsewhere.
const EXACT_NAME_BONUS: usize = 100;

const SYSTEM_PROMPT: &str = "Many of your tools are loaded lazily to save context: \
you can see their names and descriptions, but their parameter schemas are hidden \
until you ask for them. Before calling a tool whose parameters you have not yet \
loaded, call `tool_search` with a short query describing what you need (for example \
\"read file\" or \"send email\"). It returns the matching tools with their full JSON \
parameter schemas, and on your next step those tools become callable with their full \
parameters. Frequently used tools keep their full schemas and do not need to be \
searched for.";

/// [`SYSTEM_PROMPT`] for [`Reveal::InResult`]: the listed schema never
/// changes, so the model calls from the schema the search returned.
const IN_RESULT_SYSTEM_PROMPT: &str = "Many of your tools are loaded lazily to save context: \
you can see their names and descriptions, but their parameter schemas are hidden. \
Before calling a tool whose parameters you have not yet seen, call `tool_search` with a \
short query describing what you need (for example \"read file\" or \"send email\"). It \
returns the matching tools with their full JSON parameter schemas. The tool list keeps \
showing a placeholder schema for those tools; call them with arguments that follow the \
schema `tool_search` returned. Frequently used tools keep their full schemas and do not \
need to be searched for.";

/// How a tool found by `tool_search` becomes callable with its parameters.
///
/// `Redefine` (the default) puts the full schema on the tool's definition from
/// the next step, a tool change that costs a cache write on models that cannot
/// send changes inline. `InResult` never changes the definition: the stub
/// stays, the model calls from the schema in the search result, and the act
/// step validates each call against the tool's registered schema, so a bad
/// call comes back as an argument error naming the problem. Experimental: in
/// the `evals/tool-search-reveal` study GPT-6 Luna and Mistral Large 4 called
/// as well as with `Redefine`, while Muse Spark sent every number, boolean and
/// object as a string and never recovered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Reveal {
    #[default]
    Redefine,
    InResult,
}

/// Bounded discovery is opt-in. Hot/turn-control tools are accounted separately
/// from optional promoted schemas; every transition still uses ToolChanges.
#[derive(Debug, Clone, Copy)]
struct CatalogLimits {
    active_tools: usize,
    active_bytes: usize,
    result_bytes: usize,
}

impl CatalogLimits {
    fn from_config(config: &Value) -> Option<Self> {
        (config.get("bounded").and_then(Value::as_bool) == Some(true)).then(|| Self {
            active_tools: config
                .get("max_active_tools")
                .and_then(Value::as_u64)
                .unwrap_or(32) as usize,
            active_bytes: config
                .get("max_active_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(65536) as usize,
            result_bytes: config
                .get("max_result_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(32768) as usize,
        })
    }
}

const BOUNDED_SYSTEM_PROMPT: &str = "Additional tools are available through tool_search. \
Search by operation, capability, or MCP server purpose. The catalog includes MCP server \
names and descriptions. Search returns authoritative parameter schemas and makes matching \
operations available as native functions on your next step. If a deferred server is loaded, \
search again for its operation. Refine broad queries; only a bounded working set is advertised.";

impl Reveal {
    fn from_config(config: &Value) -> Option<Self> {
        match config.get("reveal")?.as_str()? {
            "redefine" => Some(Self::Redefine),
            "in_result" => Some(Self::InResult),
            _ => None,
        }
    }
}

/// Generic Tool Search capability.
///
/// Adding this capability enables client-side deferred tool loading for any
/// model. `threshold` controls the minimum number of tools before schemas are
/// deferred (default: [`DEFAULT_TOOL_SEARCH_THRESHOLD`]). `never_defer` names
/// tools that always keep their full schema (see [`Self::with_never_defer`]).
pub struct ToolSearchCapability {
    threshold: usize,
    never_defer: Arc<HashSet<String>>,
    reveal: Reveal,
}

impl ToolSearchCapability {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_TOOL_SEARCH_THRESHOLD)
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            threshold,
            never_defer: Arc::new(HashSet::new()),
            reveal: Reveal::default(),
        }
    }

    /// Choose how found tools become callable (config `reveal`:
    /// `"redefine"` or `"in_result"` overrides it).
    pub fn with_reveal(mut self, reveal: Reveal) -> Self {
        self.reveal = reveal;
        self
    }

    fn reveal(&self, config: &Value) -> Reveal {
        Reveal::from_config(config).unwrap_or(self.reveal)
    }

    /// Keep the named tools' full parameter schemas even above the deferral
    /// threshold. Use for hot-path tools (file/shell) so the agent is never
    /// forced through a `tool_search` round-trip before its first call. This is
    /// equivalent to marking each tool `DeferrablePolicy::Never`, but it can be
    /// set by an embedder that does not own the tool definitions. Names from
    /// config (`never_defer`) are merged with these at hook-build time.
    pub fn with_never_defer<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.never_defer = Arc::new(names.into_iter().map(Into::into).collect());
        self
    }

    /// Resolve the effective threshold and never-defer allowlist from config,
    /// merging the config `never_defer` array onto the constructor set.
    fn resolve_config(&self, config: &Value) -> (usize, Arc<HashSet<String>>) {
        let threshold = config
            .get("threshold")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(self.threshold);

        let extra = config.get("never_defer").and_then(|v| v.as_array());
        let never_defer = match extra {
            Some(arr) if !arr.is_empty() => {
                let mut merged: HashSet<String> = self.never_defer.as_ref().clone();
                merged.extend(arr.iter().filter_map(|v| v.as_str().map(str::to_string)));
                Arc::new(merged)
            }
            // No config override: reuse the constructor set without cloning.
            _ => self.never_defer.clone(),
        };
        (threshold, never_defer)
    }

    /// Build a `DeferSchemaHook` for a session whose transcript says `disclosure`.
    fn hook(
        threshold: usize,
        never_defer: Arc<HashSet<String>>,
        disclosure: ToolDisclosure,
        reveal: Reveal,
    ) -> Arc<dyn ToolDefinitionHook> {
        Arc::new(DeferSchemaHook {
            threshold,
            never_defer,
            disclosure,
            reveal,
            limits: None,
        })
    }
}

impl Default for ToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Capability for ToolSearchCapability {
    fn id(&self) -> &str {
        TOOL_SEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tool Search"
    }

    fn description(&self) -> &str {
        "Provider-agnostic deferred tool loading. Hides tool parameter schemas \
         until the model loads them via the tool_search tool, reducing token \
         usage for agents with many tools. Works with any model."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Пошук інструментів",
            "Відкладене завантаження інструментів незалежно від провайдера. Приховує схеми параметрів інструментів, доки модель не завантажить їх через інструмент tool_search, що зменшує використання токенів для агентів із багатьма інструментами. Працює з будь-якою моделлю.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    fn native_tool_disclosure_policy_with_config(
        &self,
        config: &Value,
        _definitions: &[ToolDefinition],
    ) -> Option<everruns_contracts::runtime::tool_surface::NativeToolDisclosurePolicy> {
        CatalogLimits::from_config(config)?;
        let (_, never_defer) = self.resolve_config(config);
        Some(
            everruns_contracts::runtime::tool_surface::NativeToolDisclosurePolicy {
                default: everruns_contracts::runtime::tool_surface::NativeToolDisclosure::Deferred,
                overrides: never_defer
                    .iter()
                    .map(|name| {
                        (
                            name.clone(),
                            everruns_contracts::runtime::tool_surface::NativeToolDisclosure::Eager,
                        )
                    })
                    .collect(),
            },
        )
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(match self.reveal {
            Reveal::Redefine => SYSTEM_PROMPT,
            Reveal::InResult => IN_RESULT_SYSTEM_PROMPT,
        })
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        if CatalogLimits::from_config(config).is_some() {
            return Some(BOUNDED_SYSTEM_PROMPT.to_owned());
        }
        Some(
            match self.reveal(config) {
                Reveal::Redefine => SYSTEM_PROMPT,
                Reveal::InResult => IN_RESULT_SYSTEM_PROMPT,
            }
            .to_string(),
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolSearchTool {
            reveal: self.reveal,
            limits: None,
        })]
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolSearchTool {
            reveal: self.reveal(config),
            limits: CatalogLimits::from_config(config),
        })]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        // No collection context, so no transcript: deferral still works, nothing
        // is revealed. Production goes through `tool_definition_hooks_with_context`.
        vec![Self::hook(
            self.threshold,
            self.never_defer.clone(),
            ToolDisclosure::default(),
            self.reveal,
        )]
    }

    fn tool_definition_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        let (threshold, never_defer) = self.resolve_config(config);
        vec![Arc::new(DeferSchemaHook {
            threshold,
            never_defer,
            disclosure: ToolDisclosure::default(),
            reveal: self.reveal(config),
            limits: CatalogLimits::from_config(config),
        })]
    }

    fn tool_definition_hooks_with_context(
        &self,
        ctx: &SystemPromptContext,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        let (threshold, never_defer) = self.resolve_config(config);
        vec![Arc::new(DeferSchemaHook {
            threshold,
            never_defer,
            disclosure: ctx.tool_disclosure.clone(),
            reveal: self.reveal(config),
            limits: CatalogLimits::from_config(config),
        })]
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if let Some(value) = config.get("bounded")
            && !value.is_boolean()
        {
            return Err("bounded must be a boolean".into());
        }
        for (key, minimum, maximum) in [
            ("max_active_tools", 1, 128),
            ("max_active_bytes", 1024, 1048576),
            ("max_result_bytes", 1024, 65536),
        ] {
            if let Some(value) = config.get(key)
                && !value
                    .as_u64()
                    .is_some_and(|value| value >= minimum && value <= maximum)
            {
                return Err(format!("{key} must be an integer in {minimum}..={maximum}"));
            }
        }
        if config.get("bounded").and_then(Value::as_bool) == Some(true)
            && matches!(self.reveal(config), Reveal::InResult)
        {
            return Err("bounded discovery requires reveal=redefine for typed native calls".into());
        }
        Ok(())
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({"type":"object", "properties": {
            "threshold":{"type":"integer","minimum":1},
            "never_defer":{"type":"array","items":{"type":"string"}},
            "reveal":{"type":"string","enum":["redefine","in_result"]},
            "bounded":{"type":"boolean","default":false},
            "max_active_tools":{"type":"integer","minimum":1,"maximum":128,"default":32},
            "max_active_bytes":{"type":"integer","minimum":1024,"maximum":1048576,"default":65536},
            "max_result_bytes":{"type":"integer","minimum":1024,"maximum":65536,"default":32768}
        }}))
    }
}

// ============================================================================
// DeferSchemaHook — strips parameter schemas from deferrable, unrevealed tools
// ============================================================================

/// Minimal open-object schema sent in place of a deferred tool's parameters.
///
/// The capability prompt owns the single instruction to use `tool_search`.
/// Repeating a tool-specific copy inside every stub scales that sentence with
/// the size of the tool catalogue and defeats much of the deferral saving.
fn deferred_stub_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
    })
}

pub(crate) struct DeferSchemaHook {
    threshold: usize,
    never_defer: Arc<HashSet<String>>,
    disclosure: ToolDisclosure,
    reveal: Reveal,
    limits: Option<CatalogLimits>,
}

impl DeferSchemaHook {
    fn eager(&self, tool: &ToolDefinition) -> bool {
        tool.name() == TOOL_SEARCH_TOOL_NAME
            || everruns_contracts::runtime::tool_surface::NATIVE_ONLY_TOOL_NAMES
                .contains(&tool.name())
            || self.never_defer.contains(tool.name())
            || matches!(tool, ToolDefinition::ClientSide(_))
            || *tool.policy() == crate::builtins::tool_types::ToolPolicy::ClientSide
            || tool.hints().stays_direct == Some(true)
            || (matches!(tool.deferrable(), DeferrablePolicy::Never)
                && crate::deferred_mcp_server_prefix(tool.name()).is_none())
    }

    fn bounded_projection(
        &self,
        tools: Vec<ToolDefinition>,
        limits: CatalogLimits,
    ) -> Vec<ToolDefinition> {
        let by_name: std::collections::HashMap<_, _> =
            tools.iter().map(|tool| (tool.name(), tool)).collect();
        let candidates = self
            .disclosure
            .reveal_order
            .iter()
            .rev()
            .chain(self.disclosure.failed.iter())
            .chain(self.disclosure.revealed.iter());
        let mut selected = HashSet::new();
        // Include the optional-definition array's delimiters and separators.
        // Eager host tools have their own budget and are never evicted here.
        let mut bytes = 2;
        for name in candidates {
            let Some(tool) = by_name.get(name.as_str()) else {
                continue;
            };
            if self.eager(tool) || selected.contains(name) || selected.len() >= limits.active_tools
            {
                continue;
            }
            let separator = usize::from(!selected.is_empty());
            if let Some(size) =
                serialized_size(tool, limits.active_bytes.saturating_sub(bytes + separator))
            {
                bytes += size + separator;
                selected.insert(name.clone());
            }
        }
        tools
            .into_iter()
            .filter(|tool| self.eager(tool) || selected.contains(tool.name()))
            .collect()
    }

    /// A tool keeps its full schema when it is the search tool itself, opts out
    /// via `DeferrablePolicy::Never`, is in the embedder's `never_defer`
    /// allowlist, or has already been revealed via `tool_search` this session
    /// (unless reveals stay in the search result).
    fn keep_full(&self, tool: &ToolDefinition) -> bool {
        let name = tool.name();
        let revealed = &self.disclosure.revealed;
        name == TOOL_SEARCH_TOOL_NAME
            || matches!(tool.deferrable(), DeferrablePolicy::Never)
            || self.never_defer.contains(name)
            || self.disclosure.failed.contains(name)
            || (self.reveal == Reveal::Redefine && revealed.contains(name))
            // A revealed deferred MCP server reveals every tool it lists.
            || revealed
                .iter()
                .any(|entry| entry.ends_with("__") && name.starts_with(entry.as_str()))
    }
}

impl ToolDefinitionHook for DeferSchemaHook {
    fn model_tool_budget(
        &self,
        tools: &[ToolDefinition],
    ) -> Option<everruns_contracts::runtime::capabilities::ModelToolBudget> {
        let limits = self.limits?;
        let eager: Vec<_> = tools.iter().filter(|tool| self.eager(tool)).collect();
        Some(everruns_contracts::runtime::capabilities::ModelToolBudget {
            max_tools: eager.len().saturating_add(limits.active_tools),
            max_bytes: serialized_size(&eager, usize::MAX)
                .unwrap_or(usize::MAX)
                .saturating_add(limits.active_bytes),
        })
    }

    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        if let Some(limits) = self.limits {
            return self.bounded_projection(tools, limits);
        }
        // Below the threshold full schemas fit comfortably; don't defer. Once a
        // session has deferred it keeps deferring: flipping every stub back to
        // its full schema when the count drops would rewrite the cached prefix.
        if !self.disclosure.deferring && tools.len() < self.threshold {
            return tools;
        }

        tools
            .into_iter()
            .map(|tool| {
                if self.keep_full(&tool) {
                    tool
                } else {
                    strip_parameters(tool)
                }
            })
            .collect()
    }

    // Mutually exclusive with hosted (openai) tool_search — see build().
    fn applies_with_native_tool_search(&self) -> bool {
        false
    }

    fn preserves_execution_scope(&self) -> bool {
        self.limits.is_some()
    }
}

/// Replace a tool's parameter schema with the deferred disclosure stub, keeping
/// name, description, policy, category, and hints intact. The original schema is
/// saved in `full_parameters` so `tool_search` can return it on demand.
fn strip_parameters(tool: ToolDefinition) -> ToolDefinition {
    match tool {
        ToolDefinition::Builtin(mut b) => {
            if b.full_parameters.is_none() {
                b.full_parameters = Some(b.parameters.clone());
            }
            b.parameters = deferred_stub_schema();
            ToolDefinition::Builtin(b)
        }
        ToolDefinition::ClientSide(mut c) => {
            if c.full_parameters.is_none() {
                c.full_parameters = Some(c.parameters.clone());
            }
            c.parameters = deferred_stub_schema();
            ToolDefinition::ClientSide(c)
        }
    }
}

// ============================================================================
// Tool: tool_search
// ============================================================================

/// Tool that returns full parameter schemas for tools matching a query and
/// records them as revealed (per session) so the schema hook restores them on
/// the next pass.
#[derive(Default)]
pub struct ToolSearchTool {
    reveal: Reveal,
    limits: Option<CatalogLimits>,
}

/// Count bytes without allocating the serialized form of an oversized schema.
fn serialized_size(value: &impl serde::Serialize, limit: usize) -> Option<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(bytes.len());
            if self.bytes > self.limit {
                return Err(std::io::Error::other("catalog byte budget exceeded"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.bytes)
}

impl ToolSearchTool {
    async fn bounded_search(
        &self,
        query: &str,
        context: &ToolContext,
        limits: CatalogLimits,
    ) -> ToolExecutionResult {
        if query.len() > 512 {
            return ToolExecutionResult::tool_error(
                "Search query exceeds 512 UTF-8 bytes; refine it.",
            );
        }
        let (Some(registry), Some(allowed)) = (&context.tool_registry, &context.visible_tool_names)
        else {
            return ToolExecutionResult::tool_error(
                "Search requires the current authorized operation registry.",
            );
        };
        let index = registry.search_index();
        let mut hits = index.search_operations(query, MAX_SEARCH_RESULTS, |name| {
            name != TOOL_SEARCH_TOOL_NAME && allowed.contains(name)
        });
        if hits.is_empty() {
            hits = index.search(query, MAX_SEARCH_RESULTS, |name| {
                name != TOOL_SEARCH_TOOL_NAME && allowed.contains(name)
            });
        }
        let mut matches = Vec::new();
        let mut loaded = Vec::new();
        let mut omitted = 0usize;
        let mut active_bytes = 2usize;
        let mut matching_sources = std::collections::BTreeSet::new();
        let mut matching_capabilities = std::collections::BTreeSet::new();
        let mut omitted_tools = Vec::new();
        let output = |matches: &Vec<Value>,
                      loaded: &Vec<String>,
                      omitted,
                      servers: &Vec<String>| {
            json!({
                "query": query, "tools": matches, "loaded": loaded, "bounded": true,
                "loading_mcp_servers": servers, "omitted_for_byte_budget": omitted,
                "message": "Selected schemas become native functions next step. After loading a server, search again for its operation. Refine source-only or oversized matches."
            })
        };
        for hit in hits
            .into_iter()
            .take(limits.active_tools.min(MAX_SEARCH_RESULTS))
        {
            let Some(definition) = registry.tool_definition(&hit.name) else {
                continue;
            };
            if !hit.tool_match {
                // A server-domain match identifies a source, not a particular
                // operation. Ask for refinement rather than promoting the
                // first alphabetical operations from a large server.
                if (hit.server_match || hit.capability_match)
                    && let Some(source) = crate::mcp_server::parse_mcp_tool_name(definition.name())
                        .map(|(source, _)| source)
                        .or_else(|| {
                            crate::deferred_mcp_server_prefix(definition.name()).map(str::to_owned)
                        })
                {
                    matching_sources.insert(source);
                }
                if hit.capability_match
                    && let Some(id) = &definition.hints().capability_id
                {
                    matching_capabilities.insert(id.clone());
                }
                continue;
            }
            let separator = usize::from(!loaded.is_empty());
            let Some(size) = serialized_size(
                &definition,
                limits.active_bytes.saturating_sub(active_bytes + separator),
            ) else {
                omitted += 1;
                omitted_tools.push(hit.name.clone());
                continue;
            };
            let value = json!({"name": definition.name(), "description": definition.description(),
                "parameters": definition.full_parameters(), "server_match": hit.server_match,
                "capability_match": hit.capability_match, "operation_match": hit.tool_match});
            matches.push(value);
            loaded.push(hit.name.clone());
            let pending: Vec<_> = loaded
                .iter()
                .filter_map(|name| crate::deferred_mcp_server_prefix(name).map(str::to_string))
                .collect();
            if serialized_size(
                &output(&matches, &loaded, MAX_SEARCH_RESULTS, &pending),
                limits.result_bytes,
            )
            .is_none()
            {
                matches.pop();
                loaded.pop();
                omitted += 1;
                omitted_tools.push(hit.name.clone());
            } else {
                active_bytes += size + separator;
            }
        }
        let servers = reveal_deferred_servers(context, &loaded).await;
        let mut result = output(&matches, &loaded, omitted, &servers);
        if matches.is_empty() {
            // Bounded source browse, never a fallback listing every operation.
            let no_source_match = matching_sources.is_empty() && matching_capabilities.is_empty();
            let sources: std::collections::BTreeSet<_> = if no_source_match {
                registry
                    .tool_names()
                    .into_iter()
                    .filter(|name| allowed.contains(*name))
                    .filter_map(|name| {
                        crate::mcp_server::parse_mcp_tool_name(name)
                            .map(|(source, _)| source)
                            .or_else(|| crate::deferred_mcp_server_prefix(name).map(str::to_string))
                    })
                    .collect()
            } else {
                matching_sources
            };
            if no_source_match {
                matching_capabilities.extend(
                    registry
                        .tool_definitions()
                        .into_iter()
                        .filter(|definition| allowed.contains(definition.name()))
                        .filter_map(|definition| definition.hints().capability_id.clone()),
                );
            }
            let mut capabilities: Vec<_> = matching_capabilities.into_iter().take(16).collect();
            let mut sources: Vec<_> = sources.into_iter().take(16).collect();
            loop {
                result["available_capabilities"] = json!(capabilities);
                result["available_servers"] = json!(sources);
                if serialized_size(&result, limits.result_bytes).is_some()
                    || (sources.is_empty() && capabilities.is_empty())
                {
                    break;
                }
                if sources.pop().is_none() {
                    capabilities.pop();
                }
            }
        }
        while !omitted_tools.is_empty() {
            result["omitted_tools"] = json!(omitted_tools);
            if serialized_size(&result, limits.result_bytes).is_some() {
                break;
            }
            omitted_tools.pop();
            if let Some(object) = result.as_object_mut() {
                object.remove("omitted_tools");
            }
        }
        if serialized_size(&result, limits.result_bytes).is_none() {
            return ToolExecutionResult::tool_error(
                "Search response exceeds its byte budget; use a shorter query.",
            );
        }
        ToolExecutionResult::success(result)
    }

    /// Rank `defs` against `query` and return the best matches (full schemas).
    ///
    /// Scoring is keyword overlap with field weighting: each whitespace-separated
    /// query term scores [`NAME_TERM_WEIGHT`] if it appears in the tool's name and
    /// [`DESC_TERM_WEIGHT`] if it only appears in the description. A query that is
    /// exactly a tool name gets [`EXACT_NAME_BONUS`] (the deferred stub nudges the
    /// model to query the exact name to load a specific tool). Ties keep registry
    /// order. Only the top score band is returned — every result is also *revealed*
    /// (un-deferred) for the session, so weak tail matches are dropped rather than
    /// permanently un-deferring loosely related tools. An empty query lists tools
    /// so the model can browse. The search tool itself is always excluded.
    fn search(defs: &[ToolDefinition], query: &str) -> Vec<Value> {
        // Strip wrapping punctuation so an exact-name query still matches when the
        // model echoes the stub's quoted/backticked tool name, e.g. `"read_file"`.
        let normalized = query
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| {
                t.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|t| !t.is_empty())
            .collect();

        let mut scored: Vec<(usize, &ToolDefinition)> = defs
            .iter()
            .filter(|d| d.name() != TOOL_SEARCH_TOOL_NAME)
            .filter_map(|d| {
                if terms.is_empty() {
                    return Some((0, d));
                }
                let name = d.name().to_lowercase();
                let desc = d.description().to_lowercase();
                let mut score = 0;
                for t in &terms {
                    if name.contains(t) {
                        score += NAME_TERM_WEIGHT;
                    } else if desc.contains(t) {
                        score += DESC_TERM_WEIGHT;
                    }
                }
                // Whole-query exact name match dominates everything else.
                if normalized == name {
                    score += EXACT_NAME_BONUS;
                }
                (score > 0).then_some((score, d))
            })
            .collect();

        // Stable sort by descending score; equal scores keep registry order.
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));

        // Keep only the top score band (>= half the best score). Scores are now
        // monotonically non-increasing, so this trims the weak tail while always
        // retaining the best match — bounding the sticky reveal set to tools that
        // are genuinely close to what the model asked for.
        let max_score = scored.first().map(|(s, _)| *s).unwrap_or(0);
        let cutoff = max_score.div_ceil(2);
        scored.retain(|(s, _)| *s >= cutoff);

        scored
            .into_iter()
            .take(MAX_SEARCH_RESULTS)
            .map(|(_, d)| {
                json!({
                    "name": d.name(),
                    "description": d.description(),
                    "parameters": d.full_parameters(),
                })
            })
            .collect()
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn narrate(
        &self,
        tool_call: &crate::builtins::tool_types::ToolCall,
        phase: crate::builtins::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::builtins::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::builtins::tool_narration::narrate_tool_search(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        TOOL_SEARCH_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Tool Search")
    }

    fn description(&self) -> &str {
        "Search the available tools by keyword and load their full parameter \
         schemas. Returns matching tools with their names, descriptions, and JSON \
         parameter schemas. Call this before using any tool whose parameters you \
         have not loaded yet."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords describing the tool or capability you need (e.g. 'read file', 'run sql', 'send message')."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    // Never defer the search tool's own schema.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(crate::builtins::tool_types::BuiltinTool {
            name: self.name().to_string(),
            display_name: self.display_name().map(str::to_string),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            policy: self.policy(),
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: self.hints(),
            full_parameters: None,
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "tool_search requires tool execution context and cannot run standalone.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if let Some(limits) = self.limits {
            return self.bounded_search(query, context, limits).await;
        }

        let Some(registry) = &context.tool_registry else {
            return ToolExecutionResult::tool_error(
                "Tool registry not available in this context. tool_search requires worker-side tool execution.",
            );
        };

        let Some(visible_tool_names) = &context.visible_tool_names else {
            return ToolExecutionResult::tool_error(
                "Visible tool allowlist not available in this context. tool_search requires turn-scoped tool definitions.",
            );
        };

        let defs: Vec<_> = registry
            .tool_definitions()
            .into_iter()
            .filter(|d| visible_tool_names.contains(d.name()))
            .collect();
        let matches = Self::search(&defs, query);

        if matches.is_empty() {
            // No keyword hits — surface the catalogue (names only) so the model
            // can refine its query instead of dead-ending.
            let names: Vec<&str> = defs
                .iter()
                .map(|d| d.name())
                .filter(|n| *n != TOOL_SEARCH_TOOL_NAME)
                .collect();
            return ToolExecutionResult::success(json!({
                "query": query,
                "tools": [],
                "message": "No tools matched the query. Try a different keyword.",
                "available_tools": names,
            }));
        }

        // The result's `loaded` (and `loading_mcp_servers`) is the record of
        // the reveal: `tool_disclosure` reads it back from the transcript, so
        // the schema hook advertises the full schema on the *registered*
        // definition next step. This is what lets a structured caller
        // actually pass arguments to them.
        let loaded: Vec<String> = matches
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        // A match on a deferred MCP server's placeholder reveals the server:
        // its tools are listed, with full schemas, from the next step on.
        let servers = reveal_deferred_servers(context, &loaded).await;

        let message = match self.reveal {
            Reveal::Redefine => {
                "Full schemas loaded; these tools are callable with their full parameters on your next step."
            }
            Reveal::InResult => {
                "Call these tools with arguments that follow the parameters shown here; the tool list keeps a placeholder schema for them."
            }
        };
        let mut result = json!({
            "query": query,
            "tools": matches,
            "loaded": loaded,
            "message": message,
        });
        if !servers.is_empty() {
            result["loading_mcp_servers"] = json!(servers);
            // A deferred server's tools were not listed at all, so they are
            // added with their full schemas in either mode.
            result["message"] = json!(
                "The listed MCP servers' tools are being loaded and are callable with their full parameters on your next step."
            );
        }
        ToolExecutionResult::success(result)
    }
}

/// Read progressive disclosure from the conversation: the tools `tool_search`
/// loaded, and whether the session has already sent deferred schemas (a
/// recorded tool list holding a stub, or any earlier search).
///
/// Only what is in `messages` counts, so a compaction checkpoint that cuts
/// the earlier transcript also forgets its reveals; the model searches again,
/// and the cached prefix restarts there anyway.
pub fn tool_disclosure(messages: &[RuntimeMessage]) -> ToolDisclosure {
    let mut disclosure = ToolDisclosure::default();
    let mut searches = HashSet::new();
    let mut calls = std::collections::HashMap::new();
    for message in messages {
        match message.role {
            RuntimeMessageRole::Agent => {
                for part in &message.content {
                    if let ContentPart::ToolCall(call) = part {
                        calls.insert(call.id.as_str(), call.name.as_str());
                        if call.name == TOOL_SEARCH_TOOL_NAME {
                            searches.insert(call.id.as_str());
                            disclosure.deferring = true;
                        }
                    }
                }
                disclosure.deferring |= recorded_stub(message);
            }
            RuntimeMessageRole::ToolResult => {
                for part in &message.content {
                    let ContentPart::ToolResult(result) = part else {
                        continue;
                    };
                    if result.error.is_some()
                        && let Some(name) = calls.get(result.tool_call_id.as_str())
                    {
                        disclosure.failed.insert((*name).to_string());
                        disclosure.reveal_order.retain(|entry| entry != name);
                        disclosure.reveal_order.push((*name).to_string());
                    }
                    let Some(result) = result
                        .result
                        .as_ref()
                        .filter(|_| searches.contains(result.tool_call_id.as_str()))
                    else {
                        continue;
                    };
                    // The hosted event store reconstructs a successful
                    // `tool.completed` text result as `Value::String`, while
                    // in-process callers retain the structured object. Search
                    // returns JSON as one text part, so decode that one layer
                    // here to make transcript-derived disclosure consistent
                    // across both histories. Keep ordinary text and malformed
                    // JSON opaque; only an object carries disclosure fields.
                    let decoded;
                    let result = match result {
                        Value::Object(_) => result,
                        Value::String(text) => {
                            decoded = serde_json::from_str::<Value>(text).ok();
                            let Some(value) = decoded.as_ref().filter(|value| value.is_object())
                            else {
                                continue;
                            };
                            value
                        }
                        _ => continue,
                    };
                    let names = |key| {
                        result
                            .get(key)
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                    };
                    let loaded: Vec<_> = names("loaded").map(str::to_string).collect();
                    for name in loaded.into_iter().rev() {
                        disclosure.revealed.insert(name.clone());
                        disclosure.reveal_order.retain(|entry| entry != &name);
                        disclosure.reveal_order.push(name);
                    }
                    if result.get("bounded").and_then(Value::as_bool) != Some(true) {
                        disclosure.revealed.extend(
                            names("loading_mcp_servers").map(|prefix| format!("mcp_{prefix}__")),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    disclosure
}

/// Whether the tool list recorded on this answer holds a deferred stub.
fn recorded_stub(message: &RuntimeMessage) -> bool {
    let Some(changes) = message
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(everruns_contracts::tool_changes::METADATA_KEY))
        .and_then(|value| serde_json::from_value::<ToolChanges>(value.clone()).ok())
    else {
        return false;
    };
    let stub = deferred_stub_schema();
    changes
        .baseline
        .iter()
        .flatten()
        .chain(&changes.added)
        .chain(&changes.redefined)
        .any(|tool| *tool.parameters() == stub)
}

/// Reveal the deferred MCP servers whose placeholders are among `loaded`,
/// returning their tool prefixes. Without session storage nothing is revealed.
async fn reveal_deferred_servers(context: &ToolContext, loaded: &[String]) -> Vec<String> {
    let prefixes: Vec<String> = loaded
        .iter()
        .filter_map(|name| crate::deferred_mcp_server_prefix(name).map(str::to_string))
        .collect();
    let Some(storage) = context
        .storage_store
        .as_ref()
        .filter(|_| !prefixes.is_empty())
    else {
        return Vec::new();
    };
    let mut revealed = Vec::new();
    for prefix in prefixes {
        match crate::reveal_deferred_mcp_server(storage.as_ref(), context.session_id, &prefix).await
        {
            Ok(()) => revealed.push(prefix),
            Err(error) => tracing::warn!(%error, prefix, "Failed to reveal deferred MCP server"),
        }
    }
    revealed
}

#[cfg(test)]
#[path = "tool_search_deferred_tests.rs"]
mod deferred_tests;

#[cfg(test)]
#[path = "tool_search_bounded_tests.rs"]
mod bounded_tests;

#[cfg(test)]
mod tests;
