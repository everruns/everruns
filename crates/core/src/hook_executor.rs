// User-defined hooks: executor backends.
//
// `HookExecutor` is the per-backend trait. v1 ships only `BashHookExecutor`,
// which routes the user-authored shell command through the session's
// `bashkit_shell` sandbox (the same FS isolation `bash` itself uses) and parses
// the structured JSON contract documented in `knowledge/runtime-resources/user-hooks.md`.
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

/// Envelope handed to every executor. For bash hooks this is serialized
/// into `$EVERRUNS_HOOK_PAYLOAD_JSON` / `$EVERRUNS_HOOK_PAYLOAD_PATH`;
/// other backends (webhook, wasm, blueprint) consume it in their own
/// format. `data` is event-specific; see `knowledge/runtime-resources/user-hooks.md` for the
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

/// Bash backend. Runs the configured command inside `bashkit_shell` against
/// the session VFS. JSON payload is delivered to the script via
/// `$EVERRUNS_HOOK_PAYLOAD_JSON` (and the same JSON written to
/// `$EVERRUNS_HOOK_PAYLOAD_PATH` on the session VFS); the script writes a
/// JSON decision to stdout. Falls back to exit-code semantics when stdout is
/// empty (Git-hook compatibility).
///
/// This struct is backend-agnostic: it serializes the payload, hands it to a
/// `BashHookDispatcher`, and parses the dispatcher's stdout/exit_code/stderr
/// into a `HookOutcome`. The production dispatcher
/// (for example `everruns_integrations_bashkit::BashkitShellHookDispatcher`) runs the command
/// through the same bashkit interpreter the `bashkit_shell` capability uses.
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

impl BashHookExecutor {
    /// Build an executor wired to the given dispatcher. This is the
    /// production constructor used by the capability collection path.
    pub fn with_dispatcher(
        command: String,
        env: std::collections::BTreeMap<String, String>,
        dispatcher: Arc<dyn BashHookDispatcher>,
    ) -> Self {
        Self {
            command,
            env,
            dispatcher: Some(dispatcher),
        }
    }
}

/// Workspace-relative directory the hook script reads the payload file from.
/// Concrete dispatchers may map this onto a different storage path (e.g.
/// the bashkit `bashkit_shell` adapter strips the `/workspace` prefix before
/// hitting the session VFS).
pub const HOOK_PAYLOAD_WORKSPACE_DIR: &str = "/workspace/.hooks";

/// Storage-relative directory used by `SessionFileSystem` impls that strip
/// the workspace prefix. The bashkit `BashkitShellHookDispatcher` writes to
/// this path and exposes the workspace-prefixed equivalent to scripts.
pub const HOOK_PAYLOAD_DIR: &str = "/.hooks";

/// Build the standard env vars every bash hook receives — the canonical
/// `EVERRUNS_HOOK_PAYLOAD_JSON` plus the convenience scalars documented in
/// `knowledge/runtime-resources/user-hooks.md`. Returns the env in declaration order so dispatcher
/// logs render deterministically.
pub fn standard_hook_env(
    payload: &HookPayload,
    payload_path: &str,
) -> Result<Vec<(String, String)>, String> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|e| format!("failed to serialize hook payload: {e}"))?;

    let mut env: Vec<(String, String)> = vec![
        ("EVERRUNS_HOOK_PAYLOAD_JSON".to_string(), payload_json),
        (
            "EVERRUNS_HOOK_PAYLOAD_PATH".to_string(),
            payload_path.to_string(),
        ),
        (
            "EVERRUNS_HOOK_EVENT".to_string(),
            payload.event.as_str().to_string(),
        ),
        (
            "EVERRUNS_HOOK_ID".to_string(),
            payload.hook_id.as_str().to_string(),
        ),
        (
            "EVERRUNS_HOOK_SESSION_ID".to_string(),
            payload.session_id.to_string(),
        ),
    ];
    if let Some(turn_id) = &payload.turn_id {
        env.push(("EVERRUNS_HOOK_TURN_ID".to_string(), turn_id.clone()));
    }
    // Tool-event convenience scalars (extracted from data.tool_name /
    // data.tool_call_id when present).
    if let Some(tool_name) = payload.data.get("tool_name").and_then(|v| v.as_str()) {
        env.push(("EVERRUNS_HOOK_TOOL_NAME".to_string(), tool_name.to_string()));
    }
    if let Some(call_id) = payload.data.get("tool_call_id").and_then(|v| v.as_str()) {
        env.push((
            "EVERRUNS_HOOK_TOOL_CALL_ID".to_string(),
            call_id.to_string(),
        ));
    }
    Ok(env)
}

/// Filename (without directory) for a payload file. Combines a sanitized
/// hook id with a fresh UUIDv7 so concurrent invocations don't collide.
pub fn payload_filename(payload: &HookPayload) -> String {
    let safe: String = payload
        .hook_id
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}-{}.json", uuid::Uuid::now_v7())
}

/// Indirection used to route bash hook invocations through the session's
/// existing `bashkit_shell` sandbox without `everruns-core`'s executor module
/// having to depend on bashkit directly. The concrete
/// `everruns_integrations_bashkit::BashkitShellHookDispatcher` is the production
/// implementation.
#[async_trait]
pub trait BashHookDispatcher: Send + Sync {
    /// Run `command` inside the session sandbox with `payload` exposed to
    /// the script via env vars and a VFS payload file. `extra_env` is the
    /// user-authored env layered on top of the dispatcher's defaults. Honor
    /// `opts.timeout_ms` and `opts.max_output_bytes`.
    ///
    /// Returns (`exit_code`, `stdout`, `stderr`) — semantically identical to
    /// what a Unix `bash -c` invocation produces.
    async fn dispatch(
        &self,
        payload: &HookPayload,
        command: &str,
        extra_env: &std::collections::BTreeMap<String, String>,
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
        let output = match dispatcher
            .dispatch(&payload, &self.command, &self.env, opts)
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

    fn assert_outcome(actual: HookOutcome, expected: HookOutcome) {
        match (actual, expected) {
            (HookOutcome::Allow, HookOutcome::Allow) => {}
            (
                HookOutcome::Block {
                    reason: a,
                    user_message: au,
                },
                HookOutcome::Block {
                    reason: e,
                    user_message: eu,
                },
            ) => {
                assert_eq!(a, e);
                assert_eq!(au, eu);
            }
            (
                HookOutcome::Mutate {
                    patch: a,
                    reason: ar,
                },
                HookOutcome::Mutate {
                    patch: e,
                    reason: er,
                },
            ) => {
                assert_eq!(a, e);
                assert_eq!(ar, er);
            }
            (HookOutcome::Error { message: a }, HookOutcome::Error { message: e }) => {
                assert_eq!(a, e)
            }
            (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
        }
    }

    #[test]
    fn empty_output_uses_exit_status_and_trimmed_stderr() {
        for stdout in ["", " \n\u{2003}"] {
            assert_outcome(
                parse_bash_output(out(0, stdout, "ignored warning")),
                HookOutcome::Allow,
            );
            for (stderr, reason) in [
                (" denied: rm -rf \n", "denied: rm -rf"),
                (" \n", "hook exited non-zero"),
            ] {
                assert_outcome(
                    parse_bash_output(out(7, stdout, stderr)),
                    HookOutcome::Block {
                        reason: reason.into(),
                        user_message: None,
                    },
                );
            }
        }
    }

    #[test]
    fn json_decisions_preserve_complete_fields_and_take_precedence_over_exit() {
        for (stdout, expected) in [
            (r#"{"decision":"allow"}"#, HookOutcome::Allow),
            (r#"{"reason":"all good"}"#, HookOutcome::Allow),
            (
                r#"{"decision":"block"}"#,
                HookOutcome::Block {
                    reason: "hook blocked".into(),
                    user_message: None,
                },
            ),
            (
                r#"{"decision":"block","reason":"blocked","user_message":"nope"}"#,
                HookOutcome::Block {
                    reason: "blocked".into(),
                    user_message: Some("nope".into()),
                },
            ),
            (
                r#"{"decision":"mutate","patch":{"arguments":{"x":1,"text":"α"}},"reason":"normalize"}"#,
                HookOutcome::Mutate {
                    patch: serde_json::json!({"arguments":{"x":1,"text":"α"}}),
                    reason: Some("normalize".into()),
                },
            ),
        ] {
            for exit in [0, 7] {
                assert_outcome(
                    parse_bash_output(out(exit, &format!(" \n{stdout}\n"), "ignored")),
                    expected.clone(),
                );
            }
        }
    }

    #[test]
    fn invalid_decisions_and_json_shapes_report_specific_failures() {
        for stdout in [
            r#"{"decision":"mutate"}"#,
            r#"{"decision":"mutate","patch":null}"#,
        ] {
            assert_outcome(
                parse_bash_output(out(0, stdout, "")),
                HookOutcome::Error {
                    message: "hook decision `mutate` missing `patch`".into(),
                },
            );
        }
        assert_outcome(
            parse_bash_output(out(0, r#"{"decision":"explode"}"#, "")),
            HookOutcome::Error {
                message: "unknown hook decision `explode`".into(),
            },
        );
        for stdout in [
            "{not json",
            r#"{"decision":42}"#,
            r#"{"decision":"block","reason":false}"#,
            "{} trailing",
        ] {
            match parse_bash_output(out(0, stdout, "")) {
                HookOutcome::Error { message } => assert!(
                    message.starts_with("hook stdout JSON parse failed:"),
                    "{message}"
                ),
                other => panic!("expected parsing error, got {other:?}"),
            }
        }
    }

    #[test]
    fn non_json_diagnostics_obey_literal_utf8_byte_boundary() {
        for (stdout, excerpt) in [
            ("hello".into(), "hello".into()),
            ("x".repeat(80), "x".repeat(80)),
            ("x".repeat(81), "x".repeat(80)),
            (format!("{}😀tail", "x".repeat(79)), "x".repeat(79)),
            (
                format!("{}😀tail", "x".repeat(76)),
                format!("{}😀", "x".repeat(76)),
            ),
        ] {
            assert_outcome(
                parse_bash_output(out(0, &stdout, "")),
                HookOutcome::Error {
                    message: format!("hook stdout is not JSON (first 80 bytes: {excerpt})"),
                },
            );
        }
    }

    fn payload() -> HookPayload {
        HookPayload {
            event: HookEvent::PreToolUse,
            hook_id: HookId("user:check".into()),
            session_id: SessionId::from_seed(1),
            turn_id: Some("turn_1".into()),
            org_id: None,
            agent_id: None,
            ts: "2026-01-01T00:00:00Z".into(),
            data: serde_json::json!({"tool_name":"bash","tool_call_id":"call_1","arguments":{"text":"quote \" α"}}),
        }
    }

    #[test]
    fn standard_environment_preserves_payload_and_omits_absent_scalars() {
        let mut payload = payload();
        let env = standard_hook_env(&payload, "/workspace/.hooks/payload.json").unwrap();
        assert_eq!(env[0].0, "EVERRUNS_HOOK_PAYLOAD_JSON");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&env[0].1).unwrap(),
            serde_json::json!({"event":"pre_tool_use","hook_id":"user:check","session_id":"session_00000000000000000000000000000001","turn_id":"turn_1","org_id":null,"agent_id":null,"ts":"2026-01-01T00:00:00Z","data":{"tool_name":"bash","tool_call_id":"call_1","arguments":{"text":"quote \" α"}}})
        );
        assert_eq!(
            env[1..]
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    "EVERRUNS_HOOK_PAYLOAD_PATH",
                    "/workspace/.hooks/payload.json"
                ),
                ("EVERRUNS_HOOK_EVENT", "pre_tool_use"),
                ("EVERRUNS_HOOK_ID", "user:check"),
                (
                    "EVERRUNS_HOOK_SESSION_ID",
                    "session_00000000000000000000000000000001"
                ),
                ("EVERRUNS_HOOK_TURN_ID", "turn_1"),
                ("EVERRUNS_HOOK_TOOL_NAME", "bash"),
                ("EVERRUNS_HOOK_TOOL_CALL_ID", "call_1"),
            ]
        );
        payload.turn_id = None;
        for data in [
            serde_json::json!({}),
            serde_json::json!({"tool_name":42,"tool_call_id":false}),
        ] {
            payload.data = data;
            let env = standard_hook_env(&payload, "/payload").unwrap();
            assert_eq!(
                env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                [
                    "EVERRUNS_HOOK_PAYLOAD_JSON",
                    "EVERRUNS_HOOK_PAYLOAD_PATH",
                    "EVERRUNS_HOOK_EVENT",
                    "EVERRUNS_HOOK_ID",
                    "EVERRUNS_HOOK_SESSION_ID"
                ]
            );
        }
    }

    #[test]
    fn payload_filenames_sanitize_hook_ids_and_separate_invocations() {
        let mut payload = payload();
        for (id, prefix) in [("../a:β\n", "___a___-"), ("safe-A_9", "safe-A_9-")] {
            payload.hook_id = HookId(id.into());
            let first = payload_filename(&payload);
            let second = payload_filename(&payload);
            assert_ne!(first, second);
            for name in [first, second] {
                let nonce = name
                    .strip_prefix(prefix)
                    .unwrap()
                    .strip_suffix(".json")
                    .unwrap();
                assert_eq!(uuid::Uuid::parse_str(nonce).unwrap().get_version_num(), 7);
                assert!(!name.contains('/'));
            }
        }
    }

    struct InspectingDispatcher {
        result: Result<BashExecOutput, String>,
    }
    #[async_trait]
    impl BashHookDispatcher for InspectingDispatcher {
        async fn dispatch(
            &self,
            received: &HookPayload,
            command: &str,
            env: &std::collections::BTreeMap<String, String>,
            opts: &ExecutorOpts,
        ) -> Result<BashExecOutput, String> {
            assert_eq!(
                serde_json::to_value(received).unwrap(),
                serde_json::to_value(payload()).unwrap()
            );
            assert_eq!(command, "run-hook --check");
            assert_eq!(
                env,
                &std::collections::BTreeMap::from([("CUSTOM".into(), "literal $VALUE".into())])
            );
            assert_eq!((opts.timeout_ms, opts.max_output_bytes), (321, 987));
            self.result.clone()
        }
    }

    #[tokio::test]
    async fn executor_forwards_inputs_and_preserves_dispatch_failures() {
        for (result, expected) in [
            (
                Ok(out(
                    0,
                    r#"{"decision":"block","reason":"blocked","user_message":"explain"}"#,
                    "",
                )),
                HookOutcome::Block {
                    reason: "blocked".into(),
                    user_message: Some("explain".into()),
                },
            ),
            (
                Err("sandbox unavailable".into()),
                HookOutcome::Error {
                    message: "sandbox unavailable".into(),
                },
            ),
        ] {
            let executor = BashHookExecutor::with_dispatcher(
                "run-hook --check".into(),
                std::collections::BTreeMap::from([("CUSTOM".into(), "literal $VALUE".into())]),
                Arc::new(InspectingDispatcher { result }),
            );
            assert_eq!(executor.kind(), "bash");
            assert_outcome(
                executor
                    .run(
                        payload(),
                        &ExecutorOpts {
                            timeout_ms: 321,
                            max_output_bytes: 987,
                        },
                    )
                    .await,
                expected,
            );
        }
        let executor = BashHookExecutor {
            command: "run-hook".into(),
            env: Default::default(),
            dispatcher: None,
        };
        assert_outcome(
            executor.run(payload(), &ExecutorOpts::default()).await,
            HookOutcome::Error {
                message: "bash hook executor has no dispatcher; runtime did not wire it".into(),
            },
        );
    }
}
