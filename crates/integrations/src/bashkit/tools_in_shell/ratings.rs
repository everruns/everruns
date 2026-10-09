//! Risk ratings for tools that say nothing about their own risk.
//!
//! Decisions:
//!
//! - **Only the gap.** A tool reached through `tools` that declares no risk
//!   hint at all (no `readOnlyHint`, `destructiveHint`, `openWorldHint` or
//!   `idempotentHint`, common for third-party MCP servers) is rated once by the
//!   deployment's decision service: "does this tool change or delete anything
//!   outside the session?". Tools that declare anything, built-ins, and tools
//!   with an explicit approval policy are never rated.
//! - **Only more cautious.** A "yes" marks the tool destructive for the
//!   approval gate, which then asks before it at the default level. A "no"
//!   changes nothing: the tool stays as un-annotated as it was, so a rating can
//!   never remove an approval an admin or the tool itself asked for.
//! - **Once per tool and schema.** The answer is cached per tool name and a
//!   hash of its description and input schema, across sessions, so a script
//!   pays for at most one rating per new tool and a changed tool is rated
//!   again. A failed or unavailable rating is not cached and changes nothing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use everruns_contracts::runtime::decisions::{DecisionQuestion, DecisionRequest};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::tool_types::{ToolDefinition, ToolHints, ToolPolicy};
use serde_json::json;

use super::catalog::Entry;

/// Ratings kept before the cache starts over; one entry per distinct tool.
const CACHE_LIMIT: usize = 4096;

/// A "yes" at or above this marks the tool destructive.
const CHANGES_THRESHOLD: f64 = 0.5;

const QUESTION: &str = "Does calling this tool change, create, send or delete anything outside \
                        the caller's own session, such as records, messages, files or settings \
                        in another system? Answer from its name, description and input schema.";

static RATINGS: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

/// The definition the approval gate should judge `entry` by: its own, or,
/// for a hint-less tool the decision service rates as changing things, the
/// same definition marked destructive.
pub(super) async fn definition(context: &ToolContext, entry: &Entry) -> ToolDefinition {
    let definition = entry.tool.to_definition();
    if !is_unrated(&definition) {
        return definition;
    }
    match rating(context, &definition).await {
        Some(true) => {
            let hints = definition.hints().clone().with_destructive(true);
            definition.with_hints(hints)
        }
        _ => definition,
    }
}

/// Whether the tool declares nothing about its risk and leaves approval to
/// the hints.
fn is_unrated(definition: &ToolDefinition) -> bool {
    let ToolHints {
        readonly,
        destructive,
        idempotent,
        open_world,
        ..
    } = definition.hints();
    readonly.is_none()
        && destructive.is_none()
        && idempotent.is_none()
        && open_world.is_none()
        && matches!(definition.policy(), ToolPolicy::Auto)
}

/// Whether the tool changes things outside the session, `None` when nobody
/// could say.
async fn rating(context: &ToolContext, definition: &ToolDefinition) -> Option<bool> {
    let service = context.decisions.as_ref().filter(|s| s.is_configured())?;
    let key = cache_key(definition);
    if let Some(known) = cache().get(&key) {
        return Some(*known);
    }
    let request = DecisionRequest::new(json!({
        "tool": definition.name(),
        "description": definition.description(),
        "input_schema": definition.parameters(),
    }))
    .ask("changes", DecisionQuestion::noul(QUESTION))
    .with_metadata("purpose", "tools_in_shell.tool_rating");
    let changes = match service.evaluate(request).await {
        Ok(outcome) => outcome
            .get("changes")
            .and_then(|answer| answer.probability_yes())
            .map(|p| p >= CHANGES_THRESHOLD)?,
        Err(error) => {
            tracing::debug!(%error, tool = definition.name(), "tool rating unavailable");
            return None;
        }
    };
    tracing::info!(
        target: "bashkit.tools",
        tool = definition.name(),
        changes,
        "rated a tool with no risk hints"
    );
    let mut cache = cache();
    if cache.len() >= CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(key, changes);
    Some(changes)
}

fn cache() -> std::sync::MutexGuard<'static, HashMap<String, bool>> {
    RATINGS.lock().unwrap_or_else(|e| e.into_inner())
}

fn cache_key(definition: &ToolDefinition) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    definition.description().hash(&mut hasher);
    definition.parameters().to_string().hash(&mut hasher);
    format!("{}:{:016x}", definition.name(), hasher.finish())
}
