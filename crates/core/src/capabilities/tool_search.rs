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
// The capability is registered once and shared across every session/agent (see
// `CapabilityRegistry::with_builtins`), so the revealed set MUST be keyed by
// session — otherwise reveals would leak across sessions and gradually disable
// deferral process-wide. `DeferSchemaHook::transform` has no session context of
// its own, so the hook captures its `session_id` at construction time via
// `Capability::tool_definition_hooks_with_context`, and the `tool_search` tool
// keys writes by `ToolContext::session_id`. Both share one process-global
// `RevealRegistry` (keyed by session, bounded by `MAX_REVEAL_SESSIONS`).
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
use crate::tool_types::{DeferrablePolicy, ToolDefinition, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use crate::typed_id::SessionId;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

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

/// Upper bound on the number of sessions tracked in the revealed registry. The
/// capability is a process-global singleton with no session-end callback, so the
/// registry evicts the oldest sessions past this bound. An evicted session
/// simply re-runs `tool_search` if it is still active — correct, just slightly
/// less optimal. Each entry holds only a handful of tool-name strings.
const MAX_REVEAL_SESSIONS: usize = 4096;

/// Session-keyed set of tool names revealed via `tool_search`. Shared (by `Arc`)
/// between the capability, its `DeferSchemaHook`s, and its `tool_search` tool so
/// a reveal during tool execution is visible to the next context assembly for
/// the *same* session. See the "Progressive disclosure" note above.
#[derive(Default)]
struct RevealRegistry {
    sets: HashMap<SessionId, HashSet<String>>,
    /// Insertion order of session keys, for bounded eviction.
    order: VecDeque<SessionId>,
}

impl RevealRegistry {
    /// Record `names` as revealed for `session`, evicting the oldest sessions if
    /// the registry has grown past `MAX_REVEAL_SESSIONS`.
    fn reveal(&mut self, session: SessionId, names: impl IntoIterator<Item = String>) {
        if !self.sets.contains_key(&session) {
            self.order.push_back(session);
            self.sets.insert(session, HashSet::new());
        }
        // Just inserted if absent, so the entry is always present here.
        if let Some(set) = self.sets.get_mut(&session) {
            set.extend(names);
        }

        while self.sets.len() > MAX_REVEAL_SESSIONS {
            match self.order.pop_front() {
                Some(old) => {
                    self.sets.remove(&old);
                }
                None => break,
            }
        }
    }

    /// Names revealed for `session` so far (empty if none / evicted).
    fn revealed(&self, session: SessionId) -> HashSet<String> {
        self.sets.get(&session).cloned().unwrap_or_default()
    }
}

type SharedReveals = Arc<Mutex<RevealRegistry>>;

/// Lock the revealed registry, recovering from a poisoned mutex rather than
/// panicking: the revealed set is an optimization and must never take down agent
/// construction or a worker.
fn lock_reveals(reveals: &SharedReveals) -> MutexGuard<'_, RevealRegistry> {
    reveals
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const SYSTEM_PROMPT: &str = "Many of your tools are loaded lazily to save context: \
you can see their names and descriptions, but their parameter schemas are hidden \
until you ask for them. Before calling a tool whose parameters you have not yet \
loaded, call `tool_search` with a short query describing what you need (for example \
\"read file\" or \"send email\"). It returns the matching tools with their full JSON \
parameter schemas, and on your next step those tools become callable with their full \
parameters. Frequently used tools keep their full schemas and do not need to be \
searched for.";

/// Generic Tool Search capability.
///
/// Adding this capability enables client-side deferred tool loading for any
/// model. `threshold` controls the minimum number of tools before schemas are
/// deferred (default: [`DEFAULT_TOOL_SEARCH_THRESHOLD`]). `never_defer` names
/// tools that always keep their full schema (see [`Self::with_never_defer`]).
pub struct ToolSearchCapability {
    threshold: usize,
    never_defer: Arc<HashSet<String>>,
    revealed: SharedReveals,
}

impl ToolSearchCapability {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_TOOL_SEARCH_THRESHOLD)
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            threshold,
            never_defer: Arc::new(HashSet::new()),
            revealed: SharedReveals::default(),
        }
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

    /// Build a `DeferSchemaHook` for `session`, sharing this capability's
    /// revealed registry.
    fn hook(
        &self,
        threshold: usize,
        never_defer: Arc<HashSet<String>>,
        session: SessionId,
    ) -> Arc<dyn ToolDefinitionHook> {
        Arc::new(DeferSchemaHook {
            threshold,
            never_defer,
            revealed: self.revealed.clone(),
            session,
        })
    }
}

impl Default for ToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

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

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolSearchTool {
            revealed: self.revealed.clone(),
        })]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        // No collection context: progressive disclosure is keyed to an ephemeral
        // session (deferral still works; reveals just won't restore). Production
        // goes through `tool_definition_hooks_with_context`.
        vec![self.hook(self.threshold, self.never_defer.clone(), SessionId::new())]
    }

    fn tool_definition_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        let (threshold, never_defer) = self.resolve_config(config);
        vec![self.hook(threshold, never_defer, SessionId::new())]
    }

    fn tool_definition_hooks_with_context(
        &self,
        ctx: &SystemPromptContext,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        let (threshold, never_defer) = self.resolve_config(config);
        vec![self.hook(threshold, never_defer, ctx.session_id)]
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
    revealed: SharedReveals,
    /// Session this hook was built for; used to read the right reveal set.
    session: SessionId,
}

impl DeferSchemaHook {
    /// A tool keeps its full schema when it is the search tool itself, opts out
    /// via `DeferrablePolicy::Never`, is in the embedder's `never_defer`
    /// allowlist, or has already been revealed via `tool_search` this session.
    fn keep_full(&self, tool: &ToolDefinition, revealed: &HashSet<String>) -> bool {
        let name = tool.name();
        name == TOOL_SEARCH_TOOL_NAME
            || matches!(tool.deferrable(), DeferrablePolicy::Never)
            || self.never_defer.contains(name)
            || revealed.contains(name)
    }
}

impl ToolDefinitionHook for DeferSchemaHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        // Below the threshold full schemas fit comfortably; don't defer.
        if tools.len() < self.threshold {
            return tools;
        }

        let revealed = lock_reveals(&self.revealed).revealed(self.session);

        tools
            .into_iter()
            .map(|tool| {
                if self.keep_full(&tool, &revealed) {
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
    revealed: SharedReveals,
}

impl ToolSearchTool {
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
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_tool_search(
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
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
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

        // Record the matched tools as revealed for this session so the schema
        // hook advertises their full schema on the *registered* definition next
        // iteration. This is what lets a structured caller actually pass
        // arguments to them.
        let loaded: Vec<String> = matches
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        if !loaded.is_empty() {
            lock_reveals(&self.revealed).reveal(context.session_id, loaded.iter().cloned());
        }

        ToolExecutionResult::success(json!({
            "query": query,
            "tools": matches,
            "loaded": loaded,
            "message": "Full schemas loaded; these tools are callable with their full parameters on your next step.",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, ToolPolicy};

    fn builtin(name: &str, description: &str, deferrable: DeferrablePolicy) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    fn many_tools(n: usize) -> Vec<ToolDefinition> {
        (0..n)
            .map(|i| {
                builtin(
                    &format!("tool_{i}"),
                    "does something",
                    DeferrablePolicy::Automatic,
                )
            })
            .collect()
    }

    /// A bare hook with an empty allowlist, a fresh registry, and a fresh session.
    fn hook(threshold: usize) -> DeferSchemaHook {
        DeferSchemaHook {
            threshold,
            never_defer: Arc::new(HashSet::new()),
            revealed: SharedReveals::default(),
            session: SessionId::new(),
        }
    }

    fn ctx_for(session: SessionId) -> SystemPromptContext {
        SystemPromptContext::without_file_store(session)
    }

    fn is_stubbed(tool: &ToolDefinition) -> bool {
        tool.parameters().get("properties").is_none()
    }

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_hook_noop_below_threshold() {
        let hook = hook(15);
        let tools = many_tools(5);
        let out = hook.transform(tools);
        // Schemas untouched below threshold.
        for t in &out {
            assert!(t.parameters().get("properties").is_some());
        }
    }

    #[test]
    fn test_hook_strips_above_threshold() {
        let hook = hook(15);
        let out = hook.transform(many_tools(20));
        for t in &out {
            // The shared capability prompt carries the progressive-disclosure
            // instruction once; each stub is only the provider-valid open object.
            assert!(t.parameters().get("properties").is_none());
            assert_eq!(t.parameters()["additionalProperties"], json!(true));
            assert!(t.parameters().get("description").is_none());
            assert!(
                t.full_parameters().get("properties").is_some(),
                "full schema should remain available for progressive disclosure"
            );
        }
    }

    #[test]
    fn test_hook_preserves_never_defer_and_search_tool() {
        let hook = hook(3);
        let mut tools = many_tools(3);
        tools.push(builtin("write_todos", "todos", DeferrablePolicy::Never));
        tools.push(ToolSearchTool::default().to_definition());

        let out = hook.transform(tools);

        let todos = out.iter().find(|t| t.name() == "write_todos").unwrap();
        assert!(
            todos.parameters().get("properties").is_some(),
            "never-defer tool keeps full schema"
        );
        let search = out
            .iter()
            .find(|t| t.name() == TOOL_SEARCH_TOOL_NAME)
            .unwrap();
        assert!(
            search.parameters().get("properties").is_some(),
            "search tool keeps full schema"
        );
        // Deferrable tools were stripped.
        let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
        assert!(deferred.parameters().get("properties").is_none());
    }

    #[test]
    fn test_never_defer_allowlist_keeps_full_schema() {
        // An embedder allowlist keeps a tool full even though its policy is
        // Automatic (i.e. the embedder does not own its definition).
        let cap = ToolSearchCapability::with_threshold(3).with_never_defer(["tool_1"]);
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(SessionId::new()), &json!({}));
        let out = hooks[0].transform(many_tools(5));

        let kept = out.iter().find(|t| t.name() == "tool_1").unwrap();
        assert!(
            !is_stubbed(kept),
            "allowlisted tool must keep its full schema"
        );
        let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
        assert!(is_stubbed(deferred), "non-allowlisted tool must defer");
    }

    #[test]
    fn test_config_never_defer_augments_constructor() {
        // Constructor allowlist plus a config-provided one are both honored.
        let cap = ToolSearchCapability::with_threshold(3).with_never_defer(["tool_0"]);
        let config = json!({ "never_defer": ["tool_2"] });
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(SessionId::new()), &config);
        let out = hooks[0].transform(many_tools(5));

        assert!(!is_stubbed(
            out.iter().find(|t| t.name() == "tool_0").unwrap()
        ));
        assert!(!is_stubbed(
            out.iter().find(|t| t.name() == "tool_2").unwrap()
        ));
        assert!(is_stubbed(
            out.iter().find(|t| t.name() == "tool_1").unwrap()
        ));
    }

    #[test]
    fn test_config_threshold_override() {
        let cap = ToolSearchCapability::with_threshold(100);
        // Config lowers the threshold so deferral activates.
        let config = json!({ "threshold": 3 });
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(SessionId::new()), &config);
        let out = hooks[0].transform(many_tools(5));
        assert!(out.iter().any(is_stubbed));
    }

    #[test]
    fn test_revealed_tool_regains_full_schema_next_pass() {
        // The end-to-end progressive-disclosure invariant: once a tool is
        // revealed for a session, its *registered* schema (not just the
        // tool_search result text) is restored on the next hook pass for that
        // session.
        let cap = ToolSearchCapability::with_threshold(3);
        let session = SessionId::new();
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(session), &json!({}));

        // First pass: tool_0 is deferred.
        let before = hooks[0].transform(many_tools(5));
        assert!(
            is_stubbed(before.iter().find(|t| t.name() == "tool_0").unwrap()),
            "precondition: tool_0 starts deferred"
        );

        // Simulate tool_search revealing it for this session.
        lock_reveals(&cap.revealed).reveal(session, ["tool_0".to_string()]);

        // Next pass (same hook, re-run by the reason atom): full schema restored.
        let after = hooks[0].transform(many_tools(5));
        assert!(
            !is_stubbed(after.iter().find(|t| t.name() == "tool_0").unwrap()),
            "revealed tool must regain its full registered schema"
        );
        assert!(
            is_stubbed(after.iter().find(|t| t.name() == "tool_1").unwrap()),
            "unrevealed tools stay deferred"
        );
    }

    #[test]
    fn test_reveals_are_isolated_per_session() {
        // A reveal in one session must not affect another session's hook (the
        // capability is a process-global singleton shared across sessions).
        let cap = ToolSearchCapability::with_threshold(3);
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        let hook_a = cap.tool_definition_hooks_with_context(&ctx_for(session_a), &json!({}));
        let hook_b = cap.tool_definition_hooks_with_context(&ctx_for(session_b), &json!({}));

        lock_reveals(&cap.revealed).reveal(session_a, ["tool_0".to_string()]);

        let out_a = hook_a[0].transform(many_tools(5));
        let out_b = hook_b[0].transform(many_tools(5));
        assert!(
            !is_stubbed(out_a.iter().find(|t| t.name() == "tool_0").unwrap()),
            "session A revealed tool_0"
        );
        assert!(
            is_stubbed(out_b.iter().find(|t| t.name() == "tool_0").unwrap()),
            "session B must not see session A's reveal"
        );
    }

    #[test]
    fn test_reveal_registry_evicts_oldest_sessions() {
        let mut reg = RevealRegistry::default();
        let first = SessionId::new();
        reg.reveal(first, ["tool_0".to_string()]);
        for _ in 0..MAX_REVEAL_SESSIONS {
            reg.reveal(SessionId::new(), ["tool_x".to_string()]);
        }
        // The oldest session was evicted once we exceeded the bound.
        assert!(reg.revealed(first).is_empty());
        assert!(reg.sets.len() <= MAX_REVEAL_SESSIONS);
    }

    #[test]
    fn test_hook_defers_mcp_tools_and_saves_full_schema() {
        // MCP tools are deferred like regular tools. The full schema is saved
        // in full_parameters so tool_search can return it on demand.
        let hook = hook(3);
        let mut tools = many_tools(3);
        tools.push(builtin(
            "mcp_docs__search",
            "search docs",
            DeferrablePolicy::Automatic,
        ));

        let out = hook.transform(tools);

        let mcp = out.iter().find(|t| t.name() == "mcp_docs__search").unwrap();
        // Stub is sent to the model (parameters stripped).
        assert!(
            mcp.parameters().get("properties").is_none(),
            "MCP tool schema is deferred"
        );
        // Full schema is preserved for tool_search to return.
        assert!(
            mcp.full_parameters().get("properties").is_some(),
            "MCP tool full schema is accessible via full_parameters()"
        );
    }

    #[test]
    fn test_search_returns_full_schema_for_deferred_tools() {
        // After DeferSchemaHook strips parameters, tool_search must still return
        // the full schema (stored in full_parameters).
        let hook = hook(1);
        let tools = vec![builtin(
            "read_file",
            "Read a file",
            DeferrablePolicy::Automatic,
        )];
        let deferred = hook.transform(tools);

        let results = ToolSearchTool::search(&deferred, "read file");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "read_file");
        // full_parameters() is used, so real schema is returned — not the stub.
        assert!(
            results[0]["parameters"].get("properties").is_some(),
            "tool_search must return the full schema, not the deferred stub"
        );
    }

    #[test]
    fn test_search_returns_full_schema_after_serde_round_trip() {
        // Durable execution serializes reason output before scheduling act. The
        // saved full schema must survive that boundary for deferred MCP proxies.
        let hook = hook(1);
        let tools = vec![builtin(
            "mcp_docs__search",
            "Search MCP docs",
            DeferrablePolicy::Automatic,
        )];
        let deferred = hook.transform(tools);
        let round_tripped: Vec<ToolDefinition> =
            serde_json::from_value(serde_json::to_value(&deferred).unwrap()).unwrap();

        let mcp = round_tripped
            .iter()
            .find(|t| t.name() == "mcp_docs__search")
            .unwrap();
        assert!(
            mcp.parameters().get("properties").is_none(),
            "visible MCP schema remains deferred after serde"
        );

        let results = ToolSearchTool::search(&round_tripped, "docs search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "mcp_docs__search");
        assert!(
            results[0]["parameters"].get("properties").is_some(),
            "tool_search must return the full MCP schema after durable serde"
        );
    }

    #[test]
    fn test_hook_opts_out_of_native_tool_search() {
        // Generic (client-side) deferral is mutually exclusive with hosted
        // tool_search; build() uses this to skip the hook when native is active.
        let hook = hook(15);
        assert!(!hook.applies_with_native_tool_search());
    }

    #[test]
    fn test_search_ranks_by_keyword_overlap() {
        let defs = vec![
            builtin(
                "read_file",
                "Read the contents of a file",
                DeferrablePolicy::Automatic,
            ),
            builtin(
                "send_email",
                "Send an email message",
                DeferrablePolicy::Automatic,
            ),
            builtin(
                "write_file",
                "Write contents to a file",
                DeferrablePolicy::Automatic,
            ),
        ];

        let results = ToolSearchTool::search(&defs, "read file");
        assert_eq!(results[0]["name"], "read_file");
        // Full parameter schema is returned, not the stub.
        assert!(results[0]["parameters"].get("properties").is_some());

        let email = ToolSearchTool::search(&defs, "email");
        assert_eq!(email.len(), 1);
        assert_eq!(email[0]["name"], "send_email");
    }

    #[test]
    fn test_search_weights_name_above_description() {
        // A name hit outranks a description-only hit for the same term, and the
        // weaker match falls outside the top score band entirely.
        let defs = vec![
            builtin("find_user", "Search the logs", DeferrablePolicy::Automatic),
            builtin("search_logs", "Find stuff", DeferrablePolicy::Automatic),
        ];
        let results = ToolSearchTool::search(&defs, "search");
        assert_eq!(results.len(), 1, "description-only match is below the band");
        assert_eq!(results[0]["name"], "search_logs");
    }

    #[test]
    fn test_search_exact_name_match_dominates() {
        // Querying the exact tool name (as the deferred stub instructs) ranks that
        // tool first and drops near-duplicates whose name merely contains it.
        let defs = vec![
            builtin(
                "read_file_lines",
                "Read selected lines",
                DeferrablePolicy::Automatic,
            ),
            builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
        ];
        let results = ToolSearchTool::search(&defs, "read_file");
        assert_eq!(results.len(), 1, "exact name match dominates the band");
        assert_eq!(results[0]["name"], "read_file");
    }

    #[test]
    fn test_search_exact_name_match_tolerates_quoting() {
        // The deferred stub and system prompt show the tool name in quotes, so the
        // model may echo a quoted/backticked name. Wrapping punctuation must not
        // defeat the exact-name bonus (otherwise it degrades to a substring match
        // and reveals near-duplicates).
        let defs = vec![
            builtin(
                "read_file_lines",
                "Read selected lines",
                DeferrablePolicy::Automatic,
            ),
            builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
        ];
        for q in ["\"read_file\"", "`read_file`", "'read_file'"] {
            let results = ToolSearchTool::search(&defs, q);
            assert_eq!(results.len(), 1, "query {q:?} should dominate");
            assert_eq!(results[0]["name"], "read_file", "query {q:?}");
        }
    }

    #[test]
    fn test_search_caps_results_and_reveal_set() {
        // A loose query that matches many tools is capped, bounding both the
        // payload and the (sticky) reveal set.
        let mut defs = Vec::new();
        for i in 0..20 {
            defs.push(builtin(
                &format!("tool_{i}"),
                "does a thing",
                DeferrablePolicy::Automatic,
            ));
        }
        let results = ToolSearchTool::search(&defs, "thing");
        assert_eq!(results.len(), MAX_SEARCH_RESULTS);
    }

    #[test]
    fn test_search_excludes_itself() {
        let defs = vec![
            ToolSearchTool::default().to_definition(),
            builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
        ];
        let results = ToolSearchTool::search(&defs, "tool_search read");
        assert!(results.iter().all(|r| r["name"] != TOOL_SEARCH_TOOL_NAME));
    }

    #[tokio::test]
    async fn test_execute_without_registry_errors() {
        let ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        let result = ToolSearchTool::default()
            .execute_with_context(json!({ "query": "file" }), &ctx)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    struct MiniTool;
    #[async_trait]
    impl Tool for MiniTool {
        fn name(&self) -> &str {
            "read_file"
        }
        fn description(&self) -> &str {
            "Read the contents of a file"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            })
        }
        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(json!({}))
        }
    }

    #[tokio::test]
    async fn test_execute_with_registry_returns_schemas() {
        use crate::tools::ToolRegistry;

        let mut registry = ToolRegistry::new();
        registry.register(MiniTool);
        registry.register(ToolSearchTool::default());

        let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        ctx.tool_registry = Some(Arc::new(registry));
        ctx.visible_tool_names = Some(Arc::new(
            ["read_file".to_string(), TOOL_SEARCH_TOOL_NAME.to_string()]
                .into_iter()
                .collect(),
        ));

        let result = ToolSearchTool::default()
            .execute_with_context(json!({ "query": "file" }), &ctx)
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let tools = value["tools"].as_array().unwrap();
        let read = tools.iter().find(|t| t["name"] == "read_file").unwrap();
        // Full schema is returned (not the deferred stub).
        assert!(read["parameters"]["properties"]["path"].is_object());
    }

    #[tokio::test]
    async fn test_search_records_reveal_and_restores_registered_schema() {
        // The cross-cutting invariant EVE-527 asked for: a tool_search call
        // reveals the matched tool for its session, and the *same* capability's
        // hook for that session then restores its registered schema on the next
        // pass.
        use crate::tools::ToolRegistry;

        let cap = ToolSearchCapability::with_threshold(3);
        let session: SessionId = uuid::Uuid::new_v4().into();
        let hooks = cap.tool_definition_hooks_with_context(&ctx_for(session), &json!({}));

        // Precondition: read_file is deferred among a surface above threshold.
        let mut surface = many_tools(4);
        surface.push(builtin(
            "read_file",
            "Read the contents of a file",
            DeferrablePolicy::Automatic,
        ));
        let before = hooks[0].transform(surface.clone());
        assert!(is_stubbed(
            before.iter().find(|t| t.name() == "read_file").unwrap()
        ));

        // Run the capability's own tool_search tool, scoped to this session.
        let mut registry = ToolRegistry::new();
        registry.register(MiniTool);
        let tool = &cap.tools()[0];
        let mut ctx = ToolContext::new(session);
        ctx.tool_registry = Some(Arc::new(registry));
        ctx.visible_tool_names = Some(Arc::new(["read_file".to_string()].into_iter().collect()));

        let result = tool
            .execute_with_context(json!({ "query": "read file" }), &ctx)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["loaded"][0], "read_file");

        // Next pass: the registered schema for read_file is restored.
        let after = hooks[0].transform(surface);
        assert!(
            !is_stubbed(after.iter().find(|t| t.name() == "read_file").unwrap()),
            "revealed tool's registered schema must be restored after tool_search"
        );
    }

    struct HiddenTool;
    #[async_trait]
    impl Tool for HiddenTool {
        fn name(&self) -> &str {
            "write_file"
        }
        fn description(&self) -> &str {
            "Write contents to a file"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            })
        }
        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(json!({}))
        }
    }

    /// Measure the prompt-size reduction from deferral on a realistic agent
    /// surface built from production capabilities. Prints a breakdown (run with
    /// `--nocapture`) and guards that deferral keeps cutting the tool-list size
    /// by a wide margin. The printed numbers back the benchmark table in
    /// `docs/capabilities/tool-search.md`; re-run this test to refresh them.
    #[test]
    fn benchmark_prompt_size_reduction() {
        use crate::capabilities::{
            Capability, CurrentTimeCapability, SessionCapability, SessionStorageCapability,
            SkillsCapability, StatelessTodoListCapability,
        };

        // A representative core surface. Environment-backed integration
        // benchmarks live with the host composition that owns those tools.
        let caps: Vec<Box<dyn Capability>> = vec![
            Box::new(CurrentTimeCapability),
            Box::new(SessionCapability),
            Box::new(SessionStorageCapability),
            Box::new(StatelessTodoListCapability),
            Box::new(SkillsCapability),
        ];

        let mut defs: Vec<ToolDefinition> = caps
            .iter()
            .flat_map(|c| c.tools())
            .map(|t| t.to_definition())
            .collect();
        // Add the search tool itself, as the live capability does.
        defs.push(ToolSearchTool::default().to_definition());

        // What the driver serializes per tool: name + description + parameters.
        let llm_view = |defs: &[ToolDefinition]| -> usize {
            defs.iter()
                .map(|d| {
                    json!({
                        "name": d.name(),
                        "description": d.description(),
                        "parameters": d.parameters(),
                    })
                    .to_string()
                    .len()
                })
                .sum()
        };

        let total = defs.len();
        let full_bytes = llm_view(&defs);

        // Raw parameter-schema bytes (what stripping actually compresses) on the
        // full surface, before deferral.
        let params_full: usize = defs.iter().map(|d| d.parameters().to_string().len()).sum();

        // This core-only surface is intentionally smaller after environment
        // integrations moved out. Trigger deferral at its actual size; host
        // composition tests cover the public default threshold.
        let threshold = total.saturating_sub(1);
        let deferred = hook(threshold).transform(defs);
        let deferred_count = deferred.iter().filter(|d| is_stubbed(d)).count();
        let deferred_bytes = llm_view(&deferred);
        let params_deferred: usize = deferred
            .iter()
            .map(|d| d.parameters().to_string().len())
            .sum();

        // Deferral must shrink the surface; guard against underflow so a
        // regression fails with a clear message instead of a subtraction panic.
        assert!(
            deferred_bytes < full_bytes && params_deferred < params_full,
            "deferral must not grow the serialized surface \
             (tool list {full_bytes}->{deferred_bytes}, params {params_full}->{params_deferred})"
        );

        let saved = full_bytes - deferred_bytes;
        let pct = (saved as f64 / full_bytes as f64) * 100.0;
        let params_pct = ((params_full - params_deferred) as f64 / params_full as f64) * 100.0;
        // ~4 chars/token is the usual rule of thumb for JSON tool schemas.
        let approx_tokens_full = full_bytes / 4;
        let approx_tokens_deferred = deferred_bytes / 4;

        eprintln!("tool-search prompt-size benchmark");
        eprintln!("  tools on surface .......... {total}");
        eprintln!("  schemas deferred .......... {deferred_count}");
        eprintln!(
            "  full tool list ............ {full_bytes} bytes (~{approx_tokens_full} tokens)"
        );
        eprintln!(
            "  deferred tool list ........ {deferred_bytes} bytes (~{approx_tokens_deferred} tokens)"
        );
        eprintln!("  tool-list saved ........... {saved} bytes ({pct:.0}%)");
        eprintln!(
            "  parameter schemas ......... {params_full} -> {params_deferred} bytes ({params_pct:.0}% smaller)"
        );

        // Sanity guard: the smaller core-only surface must still shrink
        // materially. Host composition owns the larger environment-backed
        // benchmark.
        assert!(
            total >= threshold,
            "surface should meet or exceed the default threshold ({total} < {threshold})"
        );
        assert!(
            pct > 40.0,
            "deferral should materially cut the whole tool list (was {pct:.0}%)"
        );
        assert!(
            params_pct > 65.0,
            "parameter schemas should compress substantially (was {params_pct:.0}%)"
        );
    }

    #[tokio::test]
    async fn test_execute_filters_registry_to_visible_tools() {
        use crate::tools::ToolRegistry;

        let mut registry = ToolRegistry::new();
        registry.register(MiniTool);
        registry.register(HiddenTool);
        registry.register(ToolSearchTool::default());

        let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        ctx.tool_registry = Some(Arc::new(registry));
        ctx.visible_tool_names = Some(Arc::new(
            ["read_file".to_string(), TOOL_SEARCH_TOOL_NAME.to_string()]
                .into_iter()
                .collect(),
        ));

        let result = ToolSearchTool::default()
            .execute_with_context(json!({ "query": "file" }), &ctx)
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let tools = value["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "read_file"));
        assert!(tools.iter().all(|t| t["name"] != "write_file"));

        let result = ToolSearchTool::default()
            .execute_with_context(json!({ "query": "missing" }), &ctx)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let available = value["available_tools"].as_array().unwrap();
        assert!(available.iter().any(|name| name == "read_file"));
        assert!(available.iter().all(|name| name != "write_file"));
    }
}
