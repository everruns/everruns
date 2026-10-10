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
//! - **Switchable per agent.** The capability's `rate_unlabeled_tools` config
//!   (on by default) turns ratings off for one agent. The host marks the turn's
//!   tool context with [`UnlabeledToolRatingsOff`], so the shell never reads
//!   capability config, and every path that rates (run time, the early-stop
//!   preview, `tools plan`) goes through [`definition`] and sees the mark. Off
//!   behaves exactly like a deployment without a decision service.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use everruns_contracts::runtime::decisions::{DecisionQuestion, DecisionRequest};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::tool_types::{ToolDefinition, ToolHints, ToolPolicy};
use serde_json::{Value, json};

use super::catalog::Entry;

/// Ratings kept before the cache starts over; one entry per distinct tool.
const CACHE_LIMIT: usize = 4096;

/// A "yes" at or above this marks the tool destructive.
const CHANGES_THRESHOLD: f64 = 0.5;

const QUESTION: &str = "Does calling this tool change, create, send or delete anything outside \
                        the caller's own session, such as records, messages, files or settings \
                        in another system? Answer from its name, description and input schema.";

static RATINGS: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

/// Tool context extension: this agent turned ratings off, so a tool without
/// hints is judged by its hints alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnlabeledToolRatingsOff;

/// What the approval gate should judge a tool by.
pub(super) struct Judged {
    /// The tool's own definition or, for a hint-less tool the decision
    /// service rates as changing things, the same definition marked
    /// destructive.
    pub(super) definition: ToolDefinition,
    /// The destructive mark came from a rating, not from the tool.
    pub(super) rated: bool,
}

impl Judged {
    /// Name the rating as the reason in an approval request the gate raised
    /// for this tool, so the card can say the tool declared nothing and only
    /// looks like it changes things. The gate labels the request by the
    /// destructive hint the rating added; an explicit policy match keeps its
    /// own label.
    pub(super) fn explain(&self, approval: &mut Value) {
        if self.rated && approval.get("risk").and_then(Value::as_str) == Some("destructive") {
            approval["risk"] = Value::String(RATED_RISK.to_string());
        }
    }
}

/// `risk` of an approval request raised only because of a rating.
pub(super) const RATED_RISK: &str = "rated_changes";

/// Judge `entry` for the approval gate, rating it first when it declares
/// nothing about its risk.
pub(super) async fn definition(context: &ToolContext, entry: &Entry) -> Judged {
    let definition = entry.tool.to_definition();
    if !is_unrated(&definition) || context.extension::<UnlabeledToolRatingsOff>().is_some() {
        return Judged {
            definition,
            rated: false,
        };
    }
    match rating(context, &definition).await {
        Some(true) => {
            let hints = definition.hints().clone().with_destructive(true);
            Judged {
                definition: definition.with_hints(hints),
                rated: true,
            }
        }
        _ => Judged {
            definition,
            rated: false,
        },
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
