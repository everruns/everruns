// User-defined hooks: executor backends.
//
// `HookExecutor` is the per-backend trait. v1 ships only `BashHookExecutor`,
// which routes the user-authored shell command through the session's
// `virtual_bash` sandbox (the same FS isolation `bash` itself uses) and parses
// the structured JSON contract documented in `specs/user-hooks.md`.
//
// The trait is deliberately backend-agnostic so future variants
// (`WebhookHookExecutor`, `WasmHookExecutor`, `BlueprintHookExecutor`) can
// land without changing the user-facing `UserHookSpec` shape.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::typed_id::{OrgId, SessionId};
use crate::user_hook_types::{HookEvent, HookId, HookOutcome};

// ============================================================================
// HookPayload
// ============================================================================

/// Envelope passed to the executor on stdin (or equivalent for non-bash
/// backends). `data` is event-specific; see `specs/user-hooks.md` for the
/// per-event shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPayload {
    pub event: HookEvent,
    pub hook_id: HookId,
    pub session_id: SessionId,
    pub turn_id: Option<String>,
    pub org_id: Option<OrgId>,
    pub agent_id: Option<String>,
    pub ts: String,
    pub data: serde_json::Value,
}

// ============================================================================
// ExecutorOpts
// ============================================================================

/// Per-invocation knobs honored by every executor. The adapter clamps these
/// against the global `UserHookSpec` validation; backends should treat them
/// as already-validated.
#[derive(Debug, Clone)]
pub struct ExecutorOpts {
    pub timeout_ms: u32,
    /// 64 KiB max output (stdout + stderr combined).
    pub max_output_bytes: usize,
}

impl Default for ExecutorOpts {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            max_output_bytes: 64 * 1024,
        }
    }
}

// ============================================================================
// HookExecutor trait
// ============================================================================

/// Backend that runs a single hook invocation against a single payload.
///
/// Implementations are stateless per-call; the adapter constructs the
/// `HookPayload` and calls `run` once per matching event firing.
///
/// Contract:
///
/// - On success, return `HookOutcome::Allow`, `Mutate`, or `Block` as
///   parsed from the backend's output.
/// - On failure (timeout, sandbox error, malformed output, output size
///   overrun), return `HookOutcome::Error { message }` — never panic. The
///   adapter applies the spec's `on_error` policy.
#[async_trait]
pub trait HookExecutor: Send + Sync {
    /// Stable backend identifier (matches `ExecutorSpec` tag).
    fn kind(&self) -> &'static str;

    async fn run(&self, payload: HookPayload, opts: &ExecutorOpts) -> HookOutcome;
}

// ============================================================================
// BashHookExecutor
// ============================================================================

/// Bash backend. Runs the configured command inside `virtual_bash` against
/// the session VFS. JSON payload on stdin; JSON decision on stdout. Falls
/// back to exit-code semantics when stdout is empty (Git-hook compatibility).
///
/// v1 status: this is a *skeleton* that wires the contract end-to-end at the
/// types level. The actual `virtual_bash` invocation is delegated to the
/// runtime adapter that owns the bashkit handle for a given session; this
/// executor turns the payload + command into a structured invocation the
/// adapter dispatches. Tests below cover output parsing, which is the only
/// piece this struct owns directly.
pub struct BashHookExecutor {
    /// Command the user authored (validated non-empty).
    pub command: String,
    /// Extra env vars layered onto the executor's default env.
    pub env: std::collections::BTreeMap<String, String>,
    /// Sandbox dispatcher injected by the runtime. `None` keeps the type
    /// constructible in unit tests; in production the capability collection
    /// path supplies a real dispatcher.
    pub dispatcher: Option<Arc<dyn BashHookDispatcher>>,
}

/// Indirection used to route bash hook invocations through the session's
/// existing `virtual_bash` sandbox without `everruns-core` having to depend
/// on the bashkit binding directly. The server crate (or test harness)
/// supplies an implementation when constructing the executor.
#[async_trait]
pub trait BashHookDispatcher: Send + Sync {
    /// Run `command` with `stdin_json` on stdin and the given env layered on
    /// top of the sandbox's default environment. Honor `opts.timeout_ms` and
    /// `opts.max_output_bytes`.
    ///
    /// Returns (`exit_code`, `stdout`, `stderr`) — semantically identical to
    /// what a Unix `bash -c` invocation produces.
    async fn dispatch(
        &self,
        session_id: SessionId,
        command: &str,
        env: &std::collections::BTreeMap<String, String>,
        stdin_json: &str,
        opts: &ExecutorOpts,
    ) -> Result<BashExecOutput, String>;
}

#[derive(Debug, Clone)]
pub struct BashExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
impl HookExecutor for BashHookExecutor {
    fn kind(&self) -> &'static str {
        "bash"
    }

    async fn run(&self, payload: HookPayload, opts: &ExecutorOpts) -> HookOutcome {
        let Some(dispatcher) = &self.dispatcher else {
            return HookOutcome::Error {
                message: "bash hook executor has no dispatcher; runtime did not wire it"
                    .to_string(),
            };
        };
        let session_id = payload.session_id;
        let stdin_json = match serde_json::to_string(&payload) {
            Ok(s) => s,
            Err(e) => {
                return HookOutcome::Error {
                    message: format!("failed to serialize hook payload: {e}"),
                };
            }
        };
        let output = match dispatcher
            .dispatch(session_id, &self.command, &self.env, &stdin_json, opts)
            .await
        {
            Ok(out) => out,
            Err(message) => return HookOutcome::Error { message },
        };

        parse_bash_output(output)
    }
}

/// Parse the bash backend's output per the spec contract.
///
/// 1. Empty stdout -> exit 0 = Allow, non-zero = Block (stderr as reason).
/// 2. stdout starting with `{` -> parse as JSON decision.
/// 3. Anything else -> Error (executor failed).
pub fn parse_bash_output(out: BashExecOutput) -> HookOutcome {
    let trimmed = out.stdout.trim_start();

    if trimmed.is_empty() {
        if out.exit_code == 0 {
            return HookOutcome::Allow;
        }
        let reason = if out.stderr.trim().is_empty() {
            "hook exited non-zero".to_string()
        } else {
            out.stderr.trim().to_string()
        };
        return HookOutcome::Block {
            reason,
            user_message: None,
        };
    }

    if !trimmed.starts_with('{') {
        return HookOutcome::Error {
            message: format!(
                "hook stdout is not JSON (first 80 bytes: {})",
                first_n(trimmed, 80)
            ),
        };
    }

    #[derive(Deserialize)]
    struct Decision {
        #[serde(default)]
        decision: Option<String>,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        user_message: Option<String>,
        #[serde(default)]
        patch: Option<serde_json::Value>,
    }

    let decision: Decision = match serde_json::from_str(trimmed) {
        Ok(d) => d,
        Err(e) => {
            return HookOutcome::Error {
                message: format!("hook stdout JSON parse failed: {e}"),
            };
        }
    };

    match decision.decision.as_deref().unwrap_or("allow") {
        "allow" => HookOutcome::Allow,
        "block" => HookOutcome::Block {
            reason: decision.reason.unwrap_or_else(|| "hook blocked".into()),
            user_message: decision.user_message,
        },
        "mutate" => match decision.patch {
            Some(patch) => HookOutcome::Mutate {
                patch,
                reason: decision.reason,
            },
            None => HookOutcome::Error {
                message: "hook decision `mutate` missing `patch`".into(),
            },
        },
        other => HookOutcome::Error {
            message: format!("unknown hook decision `{other}`"),
        },
    }
}

fn first_n(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        let mut end = n;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(exit: i32, stdout: &str, stderr: &str) -> BashExecOutput {
        BashExecOutput {
            exit_code: exit,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn empty_stdout_zero_exit_is_allow() {
        assert!(matches!(
            parse_bash_output(out(0, "", "")),
            HookOutcome::Allow
        ));
    }

    #[test]
    fn empty_stdout_nonzero_exit_is_block_with_stderr_reason() {
        let outcome = parse_bash_output(out(1, "", "denied: rm -rf"));
        match outcome {
            HookOutcome::Block { reason, .. } => assert_eq!(reason, "denied: rm -rf"),
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn empty_stdout_nonzero_exit_no_stderr_uses_generic_reason() {
        let outcome = parse_bash_output(out(1, "", ""));
        match outcome {
            HookOutcome::Block { reason, .. } => assert_eq!(reason, "hook exited non-zero"),
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn json_allow_decision() {
        let outcome = parse_bash_output(out(0, r#"{"decision":"allow"}"#, ""));
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[test]
    fn json_block_with_reason_and_user_message() {
        let outcome = parse_bash_output(out(
            0,
            r#"{"decision":"block","reason":"blocked","user_message":"nope"}"#,
            "",
        ));
        match outcome {
            HookOutcome::Block {
                reason,
                user_message,
            } => {
                assert_eq!(reason, "blocked");
                assert_eq!(user_message.as_deref(), Some("nope"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn json_mutate_requires_patch() {
        let no_patch = parse_bash_output(out(0, r#"{"decision":"mutate"}"#, ""));
        assert!(matches!(no_patch, HookOutcome::Error { .. }));

        let with_patch = parse_bash_output(out(
            0,
            r#"{"decision":"mutate","patch":{"arguments":{"x":1}}}"#,
            "",
        ));
        match with_patch {
            HookOutcome::Mutate { patch, .. } => {
                assert_eq!(patch["arguments"]["x"], 1);
            }
            _ => panic!("expected Mutate"),
        }
    }

    #[test]
    fn unknown_decision_is_error() {
        let outcome = parse_bash_output(out(0, r#"{"decision":"explode"}"#, ""));
        assert!(matches!(outcome, HookOutcome::Error { .. }));
    }

    #[test]
    fn non_json_stdout_is_error() {
        let outcome = parse_bash_output(out(0, "hello world", ""));
        assert!(matches!(outcome, HookOutcome::Error { .. }));
    }

    #[test]
    fn malformed_json_is_error() {
        let outcome = parse_bash_output(out(0, "{not json", ""));
        assert!(matches!(outcome, HookOutcome::Error { .. }));
    }

    #[test]
    fn missing_decision_field_defaults_to_allow() {
        let outcome = parse_bash_output(out(0, r#"{"reason":"all good"}"#, ""));
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[test]
    fn first_n_safe_on_multibyte_boundary() {
        let s = "héllo";
        assert_eq!(first_n(s, 2), "h");
        assert_eq!(first_n(s, 3), "hé");
    }
}
