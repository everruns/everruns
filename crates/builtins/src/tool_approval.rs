// Interactive tool-approval gate.
//
// Guardrails decide policy in code; this capability suspends the turn and asks
// a human. It contributes a `PreToolUseHook` that classifies each call by the
// risk the tool *declares* (`ToolHints`) rather than by guessing from its name,
// and for calls the configured `ApprovalMode` deems risky, calls out to a
// host-supplied `ToolApprover` and turns the answer into Continue/Block.
//
// Ported from yolop, where the ACP server backs the approver with the client's
// `session/request_permission`. The approver is a constructor argument rather
// than a `ToolContext` service because a host without an interactive prompt
// should not register the gate at all — a registered gate with nowhere to ask
// is either a deadlock or a silent allow, and neither is a good default.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::atoms::{PreToolUseDecision, PreToolUseHook};
use crate::capabilities::{Capability, CapabilityStatus};
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::typed_id::SessionId;
use everruns_core::tool_context::ToolContext;

pub const TOOL_APPROVAL_CAPABILITY_ID: &str = "tool_approval";

/// A host that can interactively approve or reject a tool call.
///
/// Implementations block until a human answers; the turn is genuinely
/// suspended for the duration.
#[async_trait]
pub trait ToolApprover: Send + Sync {
    /// Ask the host to approve `tool_call`. Blocks the turn until it answers.
    async fn approve(
        &self,
        session_id: SessionId,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> ApprovalDecision;
}

/// A host's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Allow this call.
    Allow,
    /// Allow this call and stop asking for this tool this session.
    AllowAlways,
    /// Reject this call.
    Reject,
    /// Reject this call and keep rejecting this tool this session.
    RejectAlways,
    /// The turn was cancelled while the request was pending.
    Cancelled,
    /// The host could not be asked (unsupported, or a transport error). The
    /// gate falls back to allowing, so a client without a permission UI keeps
    /// working autonomously rather than deadlocking every mutating tool.
    Unavailable,
}

/// How eagerly to ask for approval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMode {
    /// Ask before any state-changing action — the lowest threshold.
    Protective,
    /// Ask only before clearly destructive, irreversible, or outward-facing
    /// actions. The default.
    #[default]
    Normal,
    /// Never pause for approval; act autonomously.
    Off,
}

impl ApprovalMode {
    /// Canonical lowercase name, as it appears in capability config.
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalMode::Protective => "protective",
            ApprovalMode::Normal => "normal",
            ApprovalMode::Off => "off",
        }
    }

    /// Parse a config value, falling back to the default for anything
    /// unrecognized (config validation reports the error; the gate must not
    /// fail open or closed on a typo mid-turn).
    fn from_config(config: &serde_json::Value) -> Self {
        match config.get("mode").and_then(serde_json::Value::as_str) {
            Some("protective") => ApprovalMode::Protective,
            Some("off") => ApprovalMode::Off,
            _ => ApprovalMode::Normal,
        }
    }
}

/// How risky a tool is, derived from the tool's own [`ToolHints`] annotations
/// rather than by guessing from names.
///
/// [`ToolHints`]: crate::tool_types::ToolHints
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolRisk {
    /// Declares `readonly` — never gated.
    ReadOnly,
    /// Changes state but is not flagged destructive/outward (writes, edits, or
    /// an un-annotated tool: fail safe by treating unknown as mutating).
    Mutating,
    /// Declares `destructive` or `open_world` (deletes, force-push, publish,
    /// network side effects).
    Destructive,
}

fn classify(tool_def: &ToolDefinition) -> ToolRisk {
    let hints = tool_def.hints();
    if hints.readonly == Some(true) {
        ToolRisk::ReadOnly
    } else if hints.destructive == Some(true) || hints.open_world == Some(true) {
        ToolRisk::Destructive
    } else {
        ToolRisk::Mutating
    }
}

/// Whether a tool of the given risk needs approval at the given mode.
fn requires_approval(mode: ApprovalMode, risk: ToolRisk) -> bool {
    match mode {
        ApprovalMode::Off => false,
        ApprovalMode::Normal => matches!(risk, ToolRisk::Destructive),
        ApprovalMode::Protective => !matches!(risk, ToolRisk::ReadOnly),
    }
}

/// Blocks risky tools behind an interactive host approval.
///
/// Constructed by hosts that can service a prompt; holds the hook so a single
/// cache of "always" answers is shared across the session's turns.
pub struct ToolApprovalCapability {
    approver: Arc<dyn ToolApprover>,
    remembered: Arc<Mutex<HashMap<(SessionId, String), bool>>>,
}

impl ToolApprovalCapability {
    /// Build the gate over a host approver.
    pub fn new(approver: Arc<dyn ToolApprover>) -> Self {
        Self {
            approver,
            remembered: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn hook(&self, mode: ApprovalMode) -> Arc<dyn PreToolUseHook> {
        Arc::new(ToolApprovalHook {
            approver: self.approver.clone(),
            mode,
            remembered: self.remembered.clone(),
        })
    }
}

#[async_trait]
impl Capability for ToolApprovalCapability {
    fn id(&self) -> &str {
        TOOL_APPROVAL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tool Approval Gate"
    }

    fn description(&self) -> &str {
        "Blocks risky tools behind an interactive host approval, tuned by the approval mode."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["off", "normal", "protective"],
                    "default": "normal",
                    "title": "Approval mode",
                    "description": "off: never ask. normal: ask before tools that declare themselves destructive or outward-facing. protective: ask before anything that is not declared read-only.",
                }
            },
            "additionalProperties": false,
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let Some(object) = config.as_object() else {
            return Err("tool_approval config must be an object".to_string());
        };
        match object.get("mode") {
            None => Ok(()),
            Some(serde_json::Value::String(mode))
                if matches!(mode.as_str(), "off" | "normal" | "protective") =>
            {
                Ok(())
            }
            Some(other) => Err(format!(
                "tool_approval mode must be one of off|normal|protective, got {other}"
            )),
        }
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![self.hook(ApprovalMode::default())]
    }

    fn pre_tool_use_hooks_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![self.hook(ApprovalMode::from_config(config))]
    }
}

struct ToolApprovalHook {
    approver: Arc<dyn ToolApprover>,
    mode: ApprovalMode,
    /// "Allow always" / "reject always" answers, keyed by (session, tool name).
    /// `true` = remembered allow, `false` = remembered reject.
    remembered: Arc<Mutex<HashMap<(SessionId, String), bool>>>,
}

impl ToolApprovalHook {
    fn block(tool_call: ToolCall, reason: &str) -> PreToolUseDecision {
        PreToolUseDecision::Block {
            reason: reason.to_string(),
            user_message: Some(format!("Denied `{}` — {reason}.", tool_call.name)),
            tool_call,
        }
    }
}

#[async_trait]
impl PreToolUseHook for ToolApprovalHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        if !requires_approval(self.mode, classify(tool_def)) {
            return PreToolUseDecision::Continue(tool_call);
        }

        let key = (context.session_id, tool_call.name.clone());
        if let Some(&allowed) = self.remembered.lock().unwrap().get(&key) {
            return if allowed {
                PreToolUseDecision::Continue(tool_call)
            } else {
                Self::block(tool_call, "rejected earlier this session")
            };
        }

        match self
            .approver
            .approve(context.session_id, &tool_call, tool_def)
            .await
        {
            ApprovalDecision::Allow | ApprovalDecision::Unavailable => {
                PreToolUseDecision::Continue(tool_call)
            }
            ApprovalDecision::AllowAlways => {
                self.remembered.lock().unwrap().insert(key, true);
                PreToolUseDecision::Continue(tool_call)
            }
            ApprovalDecision::Reject => Self::block(tool_call, "rejected by user"),
            ApprovalDecision::RejectAlways => {
                self.remembered.lock().unwrap().insert(key, false);
                Self::block(tool_call, "rejected by user")
            }
            ApprovalDecision::Cancelled => Self::block(tool_call, "turn cancelled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, ToolHints};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tool_with(hints: ToolHints) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: "t".to_string(),
            display_name: None,
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints,
            full_parameters: None,
        })
    }

    fn destructive_tool() -> ToolDefinition {
        tool_with(ToolHints {
            destructive: Some(true),
            ..Default::default()
        })
    }

    fn call() -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: "t".to_string(),
            arguments: json!({}),
        }
    }

    struct ScriptedApprover {
        decision: ApprovalDecision,
        asked: AtomicUsize,
    }

    impl ScriptedApprover {
        fn new(decision: ApprovalDecision) -> Arc<Self> {
            Arc::new(Self {
                decision,
                asked: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl ToolApprover for ScriptedApprover {
        async fn approve(
            &self,
            _session_id: SessionId,
            _tool_call: &ToolCall,
            _tool_def: &ToolDefinition,
        ) -> ApprovalDecision {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.decision
        }
    }

    #[test]
    fn classify_reads_hints() {
        assert_eq!(
            classify(&tool_with(ToolHints {
                readonly: Some(true),
                ..Default::default()
            })),
            ToolRisk::ReadOnly
        );
        assert_eq!(classify(&destructive_tool()), ToolRisk::Destructive);
        assert_eq!(
            classify(&tool_with(ToolHints {
                open_world: Some(true),
                ..Default::default()
            })),
            ToolRisk::Destructive
        );
        // Un-annotated tools fail safe as mutating (gated in protective).
        assert_eq!(
            classify(&tool_with(ToolHints::default())),
            ToolRisk::Mutating
        );
        // `readonly` wins over `open_world`: a read-only network tool (e.g. web
        // search, which sets both) is never gated, not treated as outward.
        assert_eq!(
            classify(&tool_with(ToolHints {
                readonly: Some(true),
                open_world: Some(true),
                ..Default::default()
            })),
            ToolRisk::ReadOnly
        );
    }

    #[test]
    fn policy_matches_approval_semantics() {
        for risk in [
            ToolRisk::ReadOnly,
            ToolRisk::Mutating,
            ToolRisk::Destructive,
        ] {
            assert!(!requires_approval(ApprovalMode::Off, risk));
        }
        assert!(!requires_approval(ApprovalMode::Normal, ToolRisk::ReadOnly));
        assert!(!requires_approval(ApprovalMode::Normal, ToolRisk::Mutating));
        assert!(requires_approval(
            ApprovalMode::Normal,
            ToolRisk::Destructive
        ));
        assert!(!requires_approval(
            ApprovalMode::Protective,
            ToolRisk::ReadOnly
        ));
        assert!(requires_approval(
            ApprovalMode::Protective,
            ToolRisk::Mutating
        ));
        assert!(requires_approval(
            ApprovalMode::Protective,
            ToolRisk::Destructive
        ));
    }

    #[test]
    fn mode_comes_from_capability_config() {
        assert_eq!(
            ApprovalMode::from_config(&json!({"mode": "protective"})),
            ApprovalMode::Protective
        );
        assert_eq!(
            ApprovalMode::from_config(&json!({"mode": "off"})),
            ApprovalMode::Off
        );
        // Missing or unrecognized falls back to the default rather than to a
        // fail-open or fail-closed extreme.
        assert_eq!(ApprovalMode::from_config(&json!({})), ApprovalMode::Normal);
        assert_eq!(
            ApprovalMode::from_config(&json!({"mode": "nonsense"})),
            ApprovalMode::Normal
        );
    }

    #[test]
    fn config_validation_rejects_unknown_modes() {
        let capability =
            ToolApprovalCapability::new(ScriptedApprover::new(ApprovalDecision::Allow));
        assert!(
            capability
                .validate_config(&json!({"mode": "protective"}))
                .is_ok()
        );
        assert!(capability.validate_config(&serde_json::Value::Null).is_ok());
        assert!(
            capability
                .validate_config(&json!({"mode": "yolo"}))
                .is_err()
        );
        assert!(capability.validate_config(&json!("protective")).is_err());
    }

    async fn decide(
        approver: Arc<ScriptedApprover>,
        mode: ApprovalMode,
        runs: usize,
    ) -> Vec<PreToolUseDecision> {
        let capability = ToolApprovalCapability::new(approver);
        let hook = capability.hook(mode);
        let context = ToolContext::new(SessionId::new_random());
        let mut decisions = Vec::new();
        for _ in 0..runs {
            decisions.push(
                hook.before_exec(call(), &destructive_tool(), &context)
                    .await,
            );
        }
        decisions
    }

    #[tokio::test]
    async fn rejection_blocks_the_call() {
        let decisions = decide(
            ScriptedApprover::new(ApprovalDecision::Reject),
            ApprovalMode::Normal,
            1,
        )
        .await;
        assert!(matches!(decisions[0], PreToolUseDecision::Block { .. }));
    }

    #[tokio::test]
    async fn an_unreachable_approver_allows_rather_than_deadlocking() {
        // A client with no permission UI must keep working autonomously.
        let decisions = decide(
            ScriptedApprover::new(ApprovalDecision::Unavailable),
            ApprovalMode::Normal,
            1,
        )
        .await;
        assert!(matches!(decisions[0], PreToolUseDecision::Continue(_)));
    }

    #[tokio::test]
    async fn always_answers_are_remembered_for_the_session() {
        let approver = ScriptedApprover::new(ApprovalDecision::AllowAlways);
        let decisions = decide(approver.clone(), ApprovalMode::Normal, 3).await;
        assert!(
            decisions
                .iter()
                .all(|decision| matches!(decision, PreToolUseDecision::Continue(_)))
        );
        assert_eq!(
            approver.asked.load(Ordering::SeqCst),
            1,
            "the host should be asked once, then the answer reused"
        );

        let approver = ScriptedApprover::new(ApprovalDecision::RejectAlways);
        let decisions = decide(approver.clone(), ApprovalMode::Normal, 3).await;
        assert!(
            decisions
                .iter()
                .all(|decision| matches!(decision, PreToolUseDecision::Block { .. }))
        );
        assert_eq!(approver.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn one_off_answers_are_asked_every_time() {
        let approver = ScriptedApprover::new(ApprovalDecision::Allow);
        decide(approver.clone(), ApprovalMode::Normal, 3).await;
        assert_eq!(approver.asked.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn off_never_asks() {
        let approver = ScriptedApprover::new(ApprovalDecision::Reject);
        let decisions = decide(approver.clone(), ApprovalMode::Off, 1).await;
        assert!(matches!(decisions[0], PreToolUseDecision::Continue(_)));
        assert_eq!(approver.asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancellation_blocks_the_call() {
        let decisions = decide(
            ScriptedApprover::new(ApprovalDecision::Cancelled),
            ApprovalMode::Normal,
            1,
        )
        .await;
        match &decisions[0] {
            PreToolUseDecision::Block { reason, .. } => assert_eq!(reason, "turn cancelled"),
            other => panic!("expected a block, got {other:?}"),
        }
    }
}
