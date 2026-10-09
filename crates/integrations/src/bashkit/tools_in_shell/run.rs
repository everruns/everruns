//! What one shell call did with `tools`, and how a stopped script reports it.
//!
//! Decisions:
//!
//! - **A stop is final.** Once a call needs a person's approval (or the call
//!   cap is hit) the script is stopped: the builtin asks the interpreter to
//!   exit, and refuses every later `tools` call in the same shell call even
//!   where the exit does not reach (a subshell, a pipeline stage). The script
//!   is never resumed or run again: part of it may already be done and may not
//!   be safe to repeat, so the agent writes a new script from the report.
//! - **The report rides the `bash` result.** `done` lists every call that may
//!   have changed something, with its outcome; read-only calls are counted.
//!   When the stop was an approval, the gate's own `tool_approval_required`
//!   payload is merged into the result's top level, so the turn parks on the
//!   approval card exactly as for a direct call. The one-off approval binds to
//!   the tool and its exact input, so the new script's identical call runs.
//! - **Hand-off by call id.** The builtin and the `bash` tool share only the
//!   tool context, so a run is registered under the session and the `bash`
//!   call's id, and taken back once the shell returns.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::typed_id::SessionId;
use serde_json::{Map, Value, json};

/// Longest result text a `done` entry keeps.
const RESULT_PREVIEW_CHARS: usize = 400;

/// Why a script was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    NeedsApproval,
    CallLimit,
}

impl StopReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NeedsApproval => "needs_approval",
            Self::CallLimit => "call_limit",
        }
    }
}

#[derive(Debug, Clone)]
struct Stop {
    reason: StopReason,
    /// `{"tool": "github delete-branch", "input": {...}}`.
    call: Value,
    /// The approval gate's payload, when the stop is an approval.
    approval: Option<Value>,
}

#[derive(Debug, Default)]
struct State {
    done: Vec<Value>,
    read_only_calls: usize,
    stopped: Option<Stop>,
}

/// One shell call's `tools` activity.
#[derive(Debug, Default)]
pub(crate) struct Run(Mutex<State>);

impl Run {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether an earlier call stopped the script.
    pub fn is_stopped(&self) -> bool {
        self.state().stopped.is_some()
    }

    /// Stop the script at `command` with `input`. The first stop wins.
    pub fn stop(&self, reason: StopReason, command: &str, input: &Value, approval: Option<Value>) {
        let mut state = self.state();
        if state.stopped.is_none() {
            state.stopped = Some(Stop {
                reason,
                call: json!({"tool": command, "input": input}),
                approval,
            });
        }
    }

    /// Record a call that ran. A read-only call is only counted.
    pub fn record(&self, command: &str, input: &Value, read_only: bool, outcome: Outcome<'_>) {
        let mut state = self.state();
        if read_only {
            state.read_only_calls += 1;
            return;
        }
        let mut entry = json!({"tool": command, "input": input});
        match outcome {
            Outcome::Ok(result) => {
                entry["ok"] = json!(true);
                if let Some(result) = result {
                    entry["result"] = preview(result);
                }
            }
            Outcome::Failed(message) => {
                entry["ok"] = json!(false);
                entry["error"] = json!(message);
            }
        }
        state.done.push(entry);
    }

    /// Fold the report into the `bash` result. A script that ran to its end
    /// with no stop and exit 0 gets nothing extra.
    pub(crate) fn apply(&self, result: &mut Value) {
        let state = self.state();
        let failed = result.get("exit_code").and_then(Value::as_i64) != Some(0);
        let worth_reporting = state.stopped.is_some() || (failed && !state.done.is_empty());
        if !worth_reporting {
            return;
        }
        let Some(object) = result.as_object_mut() else {
            return;
        };
        let mut report = Map::new();
        report.insert("done".into(), json!(state.done));
        report.insert("read_only_calls".into(), json!(state.read_only_calls));
        if let Some(stop) = &state.stopped {
            let mut stopped = json!({"reason": stop.reason.as_str(), "call": stop.call});
            if stop.approval.is_some() {
                stopped["approval"] = json!("requested");
            }
            report.insert("stopped".into(), stopped);
            report.insert(
                "not_reached".into(),
                json!("the stopped call and everything after it; write a new script for the rest"),
            );
            object.insert("success".into(), json!(false));
        }
        object.insert("tools".into(), Value::Object(report));
        // The approval card reads the gate's payload from the top level.
        if let Some(Value::Object(approval)) =
            state.stopped.as_ref().and_then(|s| s.approval.clone())
        {
            for (key, value) in approval {
                object.insert(key, value);
            }
        }
    }
}

/// How a recorded call ended.
pub(crate) enum Outcome<'a> {
    Ok(Option<&'a Value>),
    Failed(&'a str),
}

/// A result short enough to read in the report: as is when small, else the
/// start of its text.
fn preview(result: &Value) -> Value {
    let text = match result {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() <= RESULT_PREVIEW_CHARS {
        return result.clone();
    }
    let head: String = text.chars().take(RESULT_PREVIEW_CHARS).collect();
    json!(format!("{head}… ({} chars)", text.chars().count()))
}

type RunKey = (SessionId, String);

/// Weak, so a shell call whose result is never folded (a background run, a
/// panic) leaves nothing behind once its interpreter is dropped.
static RUNS: LazyLock<Mutex<HashMap<RunKey, Weak<Run>>>> = LazyLock::new(Default::default);

fn runs() -> std::sync::MutexGuard<'static, HashMap<RunKey, Weak<Run>>> {
    RUNS.lock().unwrap_or_else(|e| e.into_inner())
}

fn key(context: &ToolContext) -> Option<RunKey> {
    context
        .tool_call_id
        .clone()
        .map(|id| (context.session_id, id))
}

/// Start tracking the shell call in `context`.
pub(crate) fn open(context: &ToolContext) -> Arc<Run> {
    let run = Arc::new(Run::default());
    if let Some(key) = key(context) {
        let mut runs = runs();
        runs.retain(|_, run| run.strong_count() > 0);
        runs.insert(key, Arc::downgrade(&run));
    }
    run
}

/// Fold the finished shell call's report into its `bash` result. Called
/// while the interpreter, which holds the run, is still alive.
pub fn finish(context: &ToolContext, mut result: Value) -> Value {
    if let Some(run) = key(context)
        .and_then(|key| runs().remove(&key))
        .and_then(|run| run.upgrade())
    {
        run.apply(&mut result);
    }
    result
}
