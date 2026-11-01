//! Monitor probes: the tool a scheduled monitor names, run by the session
//! scheduler itself on each fire, with no agent turn.
//!
//! A probe with a wake condition (`spec["wake_when"]`, see
//! `everruns_contracts::runtime::monitor_probe`) wakes the agent with one turn
//! when the condition becomes true and stays quiet otherwise; this is how a
//! poll costs fetches instead of model tokens
//! (knowledge/operations/background-limits.md, "Lean scheduled runs").
//! A probe without one only records its result on the monitor's thread.
//!
//! THREAT[TM-AGENT-018]: probes run with the session's tool context, built the
//! same way as API executor calls (`tool_context_for_ctx`): the folded
//! network access list of the harness chain, agent and session, and the host
//! egress boundary scoped to the org and session. A context that cannot be
//! built fails closed: the probe does not run and the failure is what the
//! wake condition sees.

use crate::domains::common::Ctx;
use crate::domains::session_tasks::queries::tool_context_for_ctx;
use crate::storage::StorageBackend;
use chrono::Utc;
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::egress::{EgressScope, ScopedEgressService};
use everruns_contracts::runtime::monitor_probe::{
    PROBE_STATE_SPEC_KEY, ProbeState, WAKE_WHEN_SPEC_KEY, WakeCondition, observation,
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::tools::ToolRegistry;
use everruns_core::{Caller, DefaultPermissionResolver, EgressService, ToolExecutionResult};
use serde_json::Value;
use std::sync::Arc;

/// The most of a probe result a wake message carries.
const WAKE_RESULT_CHARS: usize = 4_000;

/// Build the tool registry for monitor probes: exactly the tools named in
/// `MONITOR_PROBE_TOOLS`.
pub(crate) fn monitor_probe_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::with_monitor_probe_defaults();
    everruns_core::builtins::register_monitor_tools(&mut registry);
    for tool in everruns_integrations::web_fetch::WebFetchCapability::from_env().tools() {
        registry.register_boxed(tool);
    }
    registry
}

/// What the scheduler needs to run probes.
pub struct ProbeHost {
    db: Arc<StorageBackend>,
    tools: ToolRegistry,
    egress: Option<Arc<dyn EgressService>>,
}

impl ProbeHost {
    pub fn new(db: Arc<StorageBackend>, egress: Option<Arc<dyn EgressService>>) -> Self {
        Self::with_tools(db, monitor_probe_tool_registry(), egress)
    }

    pub(crate) fn with_tools(
        db: Arc<StorageBackend>,
        tools: ToolRegistry,
        egress: Option<Arc<dyn EgressService>>,
    ) -> Self {
        Self { db, tools, egress }
    }

    /// The session's tool context, with its network access list and egress.
    async fn context(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<everruns_core::tool_context::ToolContext, String> {
        let mut ctx = Ctx::minimal(
            Caller::internal(org_id),
            self.db.clone(),
            None,
            Arc::new(DefaultPermissionResolver),
        );
        if let Some(egress) = &self.egress {
            let scope = EgressScope {
                org_id: everruns_core::org_public_id_from_internal(org_id)
                    .parse()
                    .ok(),
                session_id: Some(session_id),
            };
            ctx =
                ctx.with_egress_service(Arc::new(ScopedEgressService::new(egress.clone(), scope)));
        }
        tool_context_for_ctx(&ctx, session_id)
            .await
            .map_err(|error| format!("the session's tool settings could not be loaded: {error}"))
    }
}

/// What running a monitor's probe produced.
pub(crate) enum ProbeOutcome {
    /// The probe ran. `thread` goes on the monitor's thread; `wake` is the
    /// text the agent's turn gets when the probe wakes it.
    Ran {
        thread: String,
        wake: Option<String>,
    },
    /// No probe to run: the caller records the plain fire placeholder and
    /// starts the schedule's normal turn.
    Skipped,
}

/// Run the probe stored in a monitor task's spec.
pub(crate) async fn run_probe(
    host: Option<&ProbeHost>,
    org_id: i64,
    session_id: SessionId,
    task: &everruns_core::SessionTask,
) -> ProbeOutcome {
    let Some(host) = host else {
        return ProbeOutcome::Skipped;
    };
    let tool_name = match task.spec.get("tool").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name,
        _ => return ProbeOutcome::Skipped,
    };
    let Some(tool) = host.tools.get(tool_name) else {
        tracing::debug!(task_id = %task.id, tool = %tool_name, "Monitor probe: tool not in probe registry, skipping");
        return ProbeOutcome::Skipped;
    };
    let wake_when = task
        .spec
        .get(WAKE_WHEN_SPEC_KEY)
        .and_then(|value| serde_json::from_value::<WakeCondition>(value.clone()).ok());
    let arguments = task
        .spec
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));

    let result = match host.context(org_id, session_id).await {
        Ok(context) => tool.execute_with_context(arguments, &context).await,
        Err(error) => ToolExecutionResult::tool_error(error),
    };
    let result: Result<Value, String> = match result {
        ToolExecutionResult::Success(value) => Ok(value),
        ToolExecutionResult::SuccessWithImages { result, .. } => Ok(result),
        ToolExecutionResult::ToolError(message) => Err(message),
        // These cannot describe what the probe saw. A plain probe falls back
        // to a normal turn, as before; a probe with a wake condition reports
        // them as its error so the condition can notice.
        other => {
            if wake_when.is_none() {
                return ProbeOutcome::Skipped;
            }
            Err(format!("{other:?}"))
        }
    };
    let now = Utc::now().to_rfc3339();

    let Some(wake_when) = wake_when else {
        let thread = match &result {
            Ok(value) => format!(
                "Probe `{tool_name}` succeeded at {now}.\n\n{}",
                result_text(value)
            ),
            Err(message) => format!("Probe `{tool_name}` failed at {now}.\n\nError: {message}"),
        };
        return ProbeOutcome::Ran { thread, wake: None };
    };

    let observed = observation(result.as_ref().map_err(String::as_str));
    let value = wake_when.observe(&observed);
    let previous = task
        .spec
        .get(PROBE_STATE_SPEC_KEY)
        .and_then(|state| serde_json::from_value::<ProbeState>(state.clone()).ok());
    let (wake, state) = wake_when.evaluate(&value, previous.as_ref());
    if let Err(error) = host
        .db
        .set_monitor_probe_state(session_id.uuid(), &task.id, serde_json::json!(state))
        .await
    {
        tracing::warn!(task_id = %task.id, error = %error, "Failed to keep a monitor probe's state");
    }

    let seen = truncate(&value.to_string(), 200);
    let thread = if wake {
        format!(
            "Probe `{tool_name}` ran at {now}: {} is met ({seen}). Waking the agent.",
            wake_when.describe()
        )
    } else {
        format!(
            "Probe `{tool_name}` ran at {now}: waiting until {} ({seen}).",
            wake_when.describe()
        )
    };
    let wake = wake.then(|| {
        let shown = match &result {
            Ok(value) => result_text(value),
            Err(message) => format!("Error: {message}"),
        };
        format!(
            "Probe result ({tool_name} at {now}):\n```\n{}\n```",
            truncate(&shown, WAKE_RESULT_CHARS)
        )
    });
    ProbeOutcome::Ran { thread, wake }
}

/// The text of a tool result: an MCP-style `content[0].text`, a plain
/// string, or the pretty-printed JSON.
fn result_text(value: &Value) -> String {
    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}… (truncated)", &text[..cut]),
        None => text.to_string(),
    }
}
