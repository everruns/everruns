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
//
// Hosted runtimes cannot block a turn on a human: the process running the act
// may be gone by the time anyone answers. `DurableToolApprover` is their
// approver (EVE-1140). It answers from decisions recorded in session storage
// and otherwise returns `Deferred`, which this hook turns into a structured
// `tool_approval_required` result; the engine parks the turn on it and the
// server's tool-approvals API records the answer for the retried call.
// Spec: knowledge/execution/tool-approval.md.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capabilities::{Capability, CapabilityStatus};
use crate::tool_hooks::{PreToolUseDecision, PreToolUseHook};
use crate::tool_types::{
    TOOL_APPROVAL_REQUIRED_CODE, ToolApprovalRequired, ToolCall, ToolDefinition, ToolPolicy,
    ToolResult,
};
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

    /// Ask with the call's [`ToolContext`] in hand.
    ///
    /// The gate always calls this one. The default ignores the context and
    /// delegates to [`approve`](Self::approve); a host whose decisions live in
    /// per-session services (session storage, for [`DurableToolApprover`])
    /// overrides it instead.
    async fn approve_in_context(
        &self,
        session_id: SessionId,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> ApprovalDecision {
        let _ = context;
        self.approve(session_id, tool_call, tool_def).await
    }

    /// Whether this approver records "always" answers durably itself.
    ///
    /// When `true` the gate does not cache them in memory; the approver
    /// answers `AllowAlways` / `RejectAlways` from its own record every time.
    fn remembers_always_decisions(&self) -> bool {
        false
    }
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
    /// gate blocks: it is only registered by hosts that can service a prompt,
    /// so this is a broken transport, not a client without a permission UI,
    /// and a gate that fails open on transport failure is not a gate.
    Unavailable,
    /// The host recorded the request and will resume the turn once a person
    /// answers. The call does not run now: the gate records a structured
    /// `tool_approval_required` result that parks the turn, and the decision
    /// reaches the retried call. Returned by durable hosts that cannot hold a
    /// turn open on a human ([`DurableToolApprover`]).
    Deferred,
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

    /// Parse a level from text, accepting the synonyms users actually say.
    ///
    /// Lenient on purpose: the same string arrives from capability config, a
    /// host's own settings file, and the model-facing `set_approval_mode`
    /// tool, and "paranoid" or "yolo" should not be a silent no-op in any of
    /// them. Unknown values return `None` so callers can report the typo
    /// rather than guess a level.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "protective" | "paranoid" | "careful" | "cautious" | "high" => {
                Some(ApprovalMode::Protective)
            }
            "normal" | "default" | "balanced" | "standard" => Some(ApprovalMode::Normal),
            "off" | "none" | "yolo" | "autonomous" => Some(ApprovalMode::Off),
            _ => None,
        }
    }

    /// Parse a capability config object, falling back to the default for
    /// anything unrecognized (config validation reports the error; a gate must
    /// not fail open or closed on a typo mid-turn).
    pub fn from_config(config: &serde_json::Value) -> Self {
        config
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .and_then(ApprovalMode::parse)
            .unwrap_or_default()
    }
}

/// Default time a person has to answer a deferred approval request before the
/// server treats it as rejected.
pub const DEFAULT_APPROVAL_TIMEOUT_SECONDS: u64 = 900;
/// Shortest configurable approval window.
pub const MIN_APPROVAL_TIMEOUT_SECONDS: u64 = 60;
/// Longest configurable approval window (one day).
pub const MAX_APPROVAL_TIMEOUT_SECONDS: u64 = 86_400;

/// Approval window from a capability config object, clamped to the allowed
/// range. Missing or malformed values fall back to the default, as `mode` does.
pub fn approval_timeout_from_config(config: &serde_json::Value) -> u64 {
    config
        .get("timeout_seconds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_SECONDS)
        .clamp(MIN_APPROVAL_TIMEOUT_SECONDS, MAX_APPROVAL_TIMEOUT_SECONDS)
}

impl std::fmt::Display for ApprovalMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How risky a tool is, derived from the tool's own [`ToolHints`] annotations
/// rather than by guessing from names.
///
/// [`ToolHints`]: crate::tool_types::ToolHints
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolRisk {
    /// Declares `readonly` without also declaring destructive or outward
    /// behavior.
    ReadOnly,
    /// Changes state but is not flagged destructive/outward (writes, edits, or
    /// an un-annotated tool: fail safe by treating unknown as mutating).
    Mutating,
    /// Declares `destructive` or `open_world` (deletes, force-push, publish,
    /// network side effects).
    Destructive,
}

/// The hint that made a tool risky, as the approval card names it.
fn risk_label(tool_def: &ToolDefinition) -> &'static str {
    let hints = tool_def.hints();
    if matches!(tool_def.policy(), ToolPolicy::RequiresApproval) {
        "requires_approval"
    } else if hints.destructive == Some(true) {
        "destructive"
    } else if hints.open_world == Some(true) {
        "open_world"
    } else {
        "mutating"
    }
}

fn classify(tool_def: &ToolDefinition) -> ToolRisk {
    let hints = tool_def.hints();
    // A tool whose definition says it requires approval is asked about at
    // every level but `off`, like a destructive one (TM-TOOL-008).
    if matches!(tool_def.policy(), ToolPolicy::RequiresApproval)
        || hints.destructive == Some(true)
        || hints.open_world == Some(true)
    {
        ToolRisk::Destructive
    } else if hints.readonly == Some(true) {
        ToolRisk::ReadOnly
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

/// A host-owned rule that decides which tool calls need approval.
///
/// Returns `true` when `tool_call` must be approved before it runs. It sees
/// the call's arguments, so a host can gate on what a call does rather than
/// only on which tool it is. It runs on the turn's task and must not block.
pub type ToolApprovalPolicy = Arc<dyn Fn(&ToolCall, &ToolDefinition) -> bool + Send + Sync>;

/// Blocks risky tools behind an interactive host approval.
///
/// Constructed by hosts that can service a prompt; holds the hook so a single
/// cache of "always" answers is shared across the session's turns. Clones
/// share the approver, policy, and that cache.
#[derive(Clone)]
pub struct ToolApprovalCapability {
    approver: Arc<dyn ToolApprover>,
    policy: Option<ToolApprovalPolicy>,
    remembered: Arc<Mutex<HashMap<(SessionId, String), bool>>>,
}

impl ToolApprovalCapability {
    /// Build the gate over a host approver.
    ///
    /// Which calls are gated follows the configured [`ApprovalMode`] and each
    /// tool's declared hints.
    pub fn new(approver: Arc<dyn ToolApprover>) -> Self {
        Self {
            approver,
            policy: None,
            remembered: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Replace hint-based classification with a host policy.
    ///
    /// With a policy the gate asks the approver exactly when `policy` returns
    /// `true`; the configured [`ApprovalMode`] and the tool's hints no longer
    /// decide. "Always" answers are still remembered per session and tool.
    pub fn with_policy(mut self, policy: ToolApprovalPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    fn hook(&self, mode: ApprovalMode) -> Arc<dyn PreToolUseHook> {
        self.hook_with_timeout(mode, DEFAULT_APPROVAL_TIMEOUT_SECONDS)
    }

    fn hook_with_timeout(
        &self,
        mode: ApprovalMode,
        timeout_seconds: u64,
    ) -> Arc<dyn PreToolUseHook> {
        Arc::new(ToolApprovalHook {
            approver: self.approver.clone(),
            mode,
            timeout_seconds,
            policy: self.policy.clone(),
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
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": MIN_APPROVAL_TIMEOUT_SECONDS,
                    "maximum": MAX_APPROVAL_TIMEOUT_SECONDS,
                    "default": DEFAULT_APPROVAL_TIMEOUT_SECONDS,
                    "title": "Approval window (seconds)",
                    "description": "How long a hosted session waits for a person to answer before the request counts as rejected.",
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
            None => {}
            Some(serde_json::Value::String(mode))
                if matches!(mode.as_str(), "off" | "normal" | "protective") => {}
            Some(other) => {
                return Err(format!(
                    "tool_approval mode must be one of off|normal|protective, got {other}"
                ));
            }
        }
        match object.get("timeout_seconds") {
            None => {}
            Some(value)
                if value.as_u64().is_some_and(|secs| {
                    (MIN_APPROVAL_TIMEOUT_SECONDS..=MAX_APPROVAL_TIMEOUT_SECONDS).contains(&secs)
                }) => {}
            Some(other) => {
                return Err(format!(
                    "tool_approval timeout_seconds must be an integer between \
                     {MIN_APPROVAL_TIMEOUT_SECONDS} and {MAX_APPROVAL_TIMEOUT_SECONDS}, got {other}"
                ));
            }
        }
        Ok(())
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![self.hook(ApprovalMode::default())]
    }

    fn pre_tool_use_hooks_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![self.hook_with_timeout(
            ApprovalMode::from_config(config),
            approval_timeout_from_config(config),
        )]
    }
}

struct ToolApprovalHook {
    approver: Arc<dyn ToolApprover>,
    mode: ApprovalMode,
    /// Window a deferred request stays open before it counts as rejected.
    timeout_seconds: u64,
    policy: Option<ToolApprovalPolicy>,
    /// "Allow always" / "reject always" answers, keyed by (session, tool name).
    /// `true` = remembered allow, `false` = remembered reject.
    remembered: Arc<Mutex<HashMap<(SessionId, String), bool>>>,
}

impl ToolApprovalHook {
    /// Cache an "always" answer, unless the approver keeps its own durable
    /// record: a long-lived hosted process serving many sessions would
    /// otherwise grow this map without bound.
    fn remember(&self, key: (SessionId, String), allowed: bool) {
        if self.approver.remembers_always_decisions() {
            return;
        }
        self.remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, allowed);
    }

    fn block(tool_call: ToolCall, reason: &str) -> PreToolUseDecision {
        PreToolUseDecision::Block {
            reason: reason.to_string(),
            user_message: Some(format!("Denied `{}` — {reason}.", tool_call.name)),
            tool_call,
        }
    }

    /// Park the call on a durable approval request instead of running it.
    fn defer(&self, tool_call: ToolCall, tool_def: &ToolDefinition) -> PreToolUseDecision {
        let asked_at = Utc::now();
        let window = i64::try_from(self.timeout_seconds)
            .unwrap_or(MAX_APPROVAL_TIMEOUT_SECONDS as i64)
            .clamp(
                MIN_APPROVAL_TIMEOUT_SECONDS as i64,
                MAX_APPROVAL_TIMEOUT_SECONDS as i64,
            );
        let expires_at = asked_at + chrono::Duration::seconds(window);
        let (arguments, arguments_truncated) = preview_arguments(&tool_call.arguments);
        let error = format!(
            "`{name}` needs a person's approval before it runs, so it did not run. The request \
             has been sent to them. If they approve, you will be told; then call `{name}` again \
             with exactly the same arguments. If nobody answers by {expires}, treat it as \
             rejected. Do not try to reach the same outcome another way.",
            name = tool_call.name,
            expires = expires_at.to_rfc3339(),
        );
        let request = ToolApprovalRequired {
            code: TOOL_APPROVAL_REQUIRED_CODE.to_string(),
            error: error.clone(),
            tool_call_id: tool_call.id.clone(),
            tool: tool_call.name.clone(),
            display_name: tool_def.display_name().map(str::to_string),
            arguments,
            arguments_truncated,
            fingerprint: approval_fingerprint(&tool_call),
            risk: if self.policy.is_some() {
                "policy".to_string()
            } else {
                risk_label(tool_def).to_string()
            },
            mode: self.mode.as_str().to_string(),
            asked_at: asked_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        };
        let result = ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::to_value(&request).unwrap_or_default()),
            images: None,
            error: Some(error),
            connection_required: None,
            raw_output: None,
        };
        PreToolUseDecision::Defer { tool_call, result }
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
        let gated = match &self.policy {
            Some(policy) => policy(&tool_call, tool_def),
            None => requires_approval(self.mode, classify(tool_def)),
        };
        if !gated {
            return PreToolUseDecision::Continue(tool_call);
        }

        let key = (context.session_id, tool_call.name.clone());
        if let Some(&allowed) = self
            .remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return if allowed {
                PreToolUseDecision::Continue(tool_call)
            } else {
                Self::block(tool_call, "rejected earlier this session")
            };
        }

        match self
            .approver
            .approve_in_context(context.session_id, &tool_call, tool_def, context)
            .await
        {
            ApprovalDecision::Allow => PreToolUseDecision::Continue(tool_call),
            ApprovalDecision::AllowAlways => {
                self.remember(key, true);
                PreToolUseDecision::Continue(tool_call)
            }
            ApprovalDecision::Reject => Self::block(tool_call, "rejected by user"),
            ApprovalDecision::RejectAlways => {
                self.remember(key, false);
                Self::block(tool_call, "rejected by user")
            }
            ApprovalDecision::Cancelled => Self::block(tool_call, "turn cancelled"),
            ApprovalDecision::Unavailable => Self::block(tool_call, "approval unavailable"),
            ApprovalDecision::Deferred => self.defer(tool_call, tool_def),
        }
    }
}

// ============================================================================
// Durable approvals (hosted runtimes)
// ============================================================================

/// Session-storage prefix every durable approval record lives under.
///
/// Reserved from the model-facing `kv_store` tool and the session storage
/// listing (`is_internal_session_kv_key`), so neither a model nor a tool can
/// mint an approval for itself. Only the server's tool-approvals API writes it.
pub use everruns_core::capabilities::TOOL_APPROVAL_KV_PREFIX;

/// How long a one-off answer waits for the retried call before it lapses.
///
/// A one-off approval is meant for the call the person just looked at, retried
/// in the turn that resumes; one left lying around should not quietly let an
/// identical call through much later.
pub const ONE_OFF_DECISION_TTL_SECONDS: i64 = 3_600;

/// Arguments preview budget for the approval card, in serialized bytes.
const ARGUMENTS_PREVIEW_BYTES: usize = 8 * 1024;

/// Digest of a call's tool name and exact arguments.
///
/// A one-off approval is recorded against it, so it lets through only the call
/// a person actually saw. Unlike the loop-detection fingerprint nothing is
/// ignored or whitespace-normalized: every byte of the arguments is part of
/// what was approved. Object keys are sorted so a re-serialized retry with the
/// same content matches.
pub fn approval_fingerprint(tool_call: &ToolCall) -> String {
    fn canonical(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort();
                let mut sorted = serde_json::Map::new();
                for key in keys {
                    sorted.insert(key.clone(), canonical(&object[key]));
                }
                serde_json::Value::Object(sorted)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(canonical).collect())
            }
            other => other.clone(),
        }
    }
    let encoded = serde_json::to_vec(&serde_json::json!({
        "tool": tool_call.name,
        "arguments": canonical(&tool_call.arguments),
    }))
    .unwrap_or_default();
    let digest = Sha256::digest(encoded);
    let mut hex = String::with_capacity(7 + digest.len() * 2);
    hex.push_str("sha256:");
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Bounded copy of a call's arguments for a person to review.
fn preview_arguments(arguments: &serde_json::Value) -> (serde_json::Value, bool) {
    let serialized = serde_json::to_string(arguments).unwrap_or_default();
    if serialized.len() <= ARGUMENTS_PREVIEW_BYTES {
        return (arguments.clone(), false);
    }
    let mut end = ARGUMENTS_PREVIEW_BYTES;
    while end > 0 && !serialized.is_char_boundary(end) {
        end -= 1;
    }
    (
        serde_json::Value::String(serialized[..end].to_string()),
        true,
    )
}

fn fold_key_part(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Session-storage key of the "always" rule for `tool`.
///
/// Folding can collide in principle, which is why [`StoredToolApproval`]
/// repeats the tool name and the reader re-checks it.
pub fn always_decision_storage_key(tool: &str) -> String {
    format!("{TOOL_APPROVAL_KV_PREFIX}always/{}", fold_key_part(tool))
}

/// Session-storage key of a one-off answer for the call with `fingerprint`.
pub fn one_off_decision_storage_key(fingerprint: &str) -> String {
    format!(
        "{TOOL_APPROVAL_KV_PREFIX}once/{}",
        fold_key_part(fingerprint)
    )
}

/// A person's answer, as the tool-approvals API records it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToolApproval {
    /// Tool the answer is for, re-checked on read.
    pub tool: String,
    /// The approved call's [`approval_fingerprint`]; `None` for an "always"
    /// rule, which covers every call of the tool in the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// `true` lets the call run, `false` refuses it.
    pub allow: bool,
    /// When the person answered.
    pub decided_at: DateTime<Utc>,
    /// When a one-off answer lapses unused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl StoredToolApproval {
    /// An "always" rule for every call of `tool` in the session.
    pub fn always(tool: &str, allow: bool, now: DateTime<Utc>) -> Self {
        Self {
            tool: tool.to_string(),
            fingerprint: None,
            allow,
            decided_at: now,
            expires_at: None,
        }
    }

    /// A one-off answer for exactly the call with `fingerprint`.
    pub fn one_off(tool: &str, fingerprint: &str, allow: bool, now: DateTime<Utc>) -> Self {
        Self {
            tool: tool.to_string(),
            fingerprint: Some(fingerprint.to_string()),
            allow,
            decided_at: now,
            expires_at: Some(now + chrono::Duration::seconds(ONE_OFF_DECISION_TTL_SECONDS)),
        }
    }
}

/// [`ToolApprover`] for hosted runtimes, backed by session storage.
///
/// It never blocks on a person. It answers from what the tool-approvals API
/// recorded — an "always" rule for the tool, then a one-off answer for this
/// exact call, taken so it is used once — and otherwise returns
/// [`ApprovalDecision::Deferred`] so the turn parks. Because every decision
/// lives in session storage, a turn resumed in another worker process, after a
/// restart, finds the answer its own request received.
///
/// Fails closed: no storage in the call's context, or a storage error, is
/// [`ApprovalDecision::Unavailable`], which blocks the call.
#[derive(Debug, Default, Clone, Copy)]
pub struct DurableToolApprover;

impl DurableToolApprover {
    async fn decide(
        store: &dyn everruns_core::session_services::SessionStorageStore,
        session_id: SessionId,
        tool_call: &ToolCall,
    ) -> Result<ApprovalDecision, String> {
        if let Some(raw) = store
            .get_value(session_id, &always_decision_storage_key(&tool_call.name))
            .await
            .map_err(|error| error.to_string())?
            && let Ok(record) = serde_json::from_str::<StoredToolApproval>(&raw)
            && record.tool == tool_call.name
            && record.fingerprint.is_none()
        {
            return Ok(if record.allow {
                ApprovalDecision::AllowAlways
            } else {
                ApprovalDecision::RejectAlways
            });
        }

        let fingerprint = approval_fingerprint(tool_call);
        // THREAT[TM-TOOL-008]: destructive read, so one approval lets exactly
        // one call through even when retries race.
        if let Some(raw) = store
            .take_value(session_id, &one_off_decision_storage_key(&fingerprint))
            .await
            .map_err(|error| error.to_string())?
            && let Ok(record) = serde_json::from_str::<StoredToolApproval>(&raw)
            && record.tool == tool_call.name
            && record.fingerprint.as_deref() == Some(fingerprint.as_str())
            && record.expires_at.is_none_or(|expires| Utc::now() < expires)
        {
            return Ok(if record.allow {
                ApprovalDecision::Allow
            } else {
                ApprovalDecision::Reject
            });
        }

        Ok(ApprovalDecision::Deferred)
    }
}

#[async_trait]
impl ToolApprover for DurableToolApprover {
    async fn approve(
        &self,
        _session_id: SessionId,
        _tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> ApprovalDecision {
        // Decisions live in session storage, which only the call's context
        // carries. Without it nobody can be asked.
        ApprovalDecision::Unavailable
    }

    fn remembers_always_decisions(&self) -> bool {
        true
    }

    async fn approve_in_context(
        &self,
        session_id: SessionId,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> ApprovalDecision {
        if context
            .cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return ApprovalDecision::Cancelled;
        }
        let Some(store) = context.storage_store.as_ref() else {
            tracing::warn!(
                session_id = %session_id,
                tool_name = %tool_call.name,
                "tool approval has no session storage; blocking the call"
            );
            return ApprovalDecision::Unavailable;
        };
        match Self::decide(store.as_ref(), session_id, tool_call).await {
            Ok(decision) => decision,
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    tool_name = %tool_call.name,
                    %error,
                    "tool approval could not read its decisions; blocking the call"
                );
                ApprovalDecision::Unavailable
            }
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
        // Risky hints take precedence over readonly so outward-facing tools
        // cannot bypass approval by also declaring themselves read-only.
        // Hints come from the tool, including untrusted MCP servers, so a
        // contradictory pair is an escape attempt, not a description.
        assert_eq!(
            classify(&tool_with(ToolHints {
                readonly: Some(true),
                open_world: Some(true),
                ..Default::default()
            })),
            ToolRisk::Destructive
        );
        assert_eq!(
            classify(&tool_with(ToolHints {
                readonly: Some(true),
                destructive: Some(true),
                ..Default::default()
            })),
            ToolRisk::Destructive
        );
    }

    #[test]
    fn a_definition_that_requires_approval_is_gated_like_a_destructive_tool() {
        let mut tool = tool_with(ToolHints {
            readonly: Some(true),
            ..Default::default()
        });
        if let ToolDefinition::Builtin(builtin) = &mut tool {
            builtin.policy = ToolPolicy::RequiresApproval;
        }
        assert_eq!(classify(&tool), ToolRisk::Destructive);
        assert_eq!(risk_label(&tool), "requires_approval");
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
    async fn readonly_outward_tool_requires_approval() {
        let approver = ScriptedApprover::new(ApprovalDecision::Reject);
        let capability = ToolApprovalCapability::new(approver.clone());
        let context = ToolContext::new(SessionId::new_random());
        let tool = tool_with(ToolHints {
            readonly: Some(true),
            open_world: Some(true),
            ..Default::default()
        });

        let decision = capability
            .hook(ApprovalMode::Normal)
            .before_exec(call(), &tool, &context)
            .await;

        assert!(matches!(decision, PreToolUseDecision::Block { .. }));
        assert_eq!(approver.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unreachable_approver_blocks_the_call() {
        let decisions = decide(
            ScriptedApprover::new(ApprovalDecision::Unavailable),
            ApprovalMode::Normal,
            1,
        )
        .await;
        assert!(matches!(decisions[0], PreToolUseDecision::Block { .. }));
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
    #[tokio::test]
    async fn a_policy_decides_instead_of_hints_and_mode() {
        // The policy gates only calls whose arguments say "drop", even on a
        // tool that declares itself read-only and with the mode off.
        let approver = ScriptedApprover::new(ApprovalDecision::Reject);
        let policy: ToolApprovalPolicy = Arc::new(|call: &ToolCall, _def: &ToolDefinition| {
            call.arguments["sql"]
                .as_str()
                .is_some_and(|sql| sql.contains("drop"))
        });
        let capability = ToolApprovalCapability::new(approver.clone()).with_policy(policy);
        let hook = capability.hook(ApprovalMode::Off);
        let context = ToolContext::new(SessionId::new_random());
        let readonly = tool_with(ToolHints {
            readonly: Some(true),
            ..Default::default()
        });
        let mut risky = call();
        risky.arguments = json!({ "sql": "drop table users" });
        let mut safe = call();
        safe.arguments = json!({ "sql": "select 1" });

        let decision = hook.before_exec(safe, &readonly, &context).await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        assert_eq!(approver.asked.load(Ordering::SeqCst), 0);

        match hook.before_exec(risky, &readonly, &context).await {
            PreToolUseDecision::Block { reason, .. } => assert_eq!(reason, "rejected by user"),
            other => panic!("expected a block, got {other:?}"),
        }
        assert_eq!(approver.asked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_policy_that_declines_skips_even_destructive_tools() {
        let approver = ScriptedApprover::new(ApprovalDecision::Reject);
        let capability = ToolApprovalCapability::new(approver.clone())
            .with_policy(Arc::new(|_: &ToolCall, _: &ToolDefinition| false));
        let context = ToolContext::new(SessionId::new_random());
        let decision = capability
            .hook(ApprovalMode::Protective)
            .before_exec(call(), &destructive_tool(), &context)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        assert_eq!(approver.asked.load(Ordering::SeqCst), 0);
    }

    // ------------------------------------------------------------------
    // Durable approvals (EVE-1140)
    // ------------------------------------------------------------------

    mod durable {
        use super::*;
        use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
        use everruns_provider::error::Result as StoreResult;

        /// Session storage shared by every "process" in a test, the way the
        /// database is shared by every worker.
        #[derive(Default)]
        struct MemoryStore {
            values: Mutex<HashMap<String, String>>,
            fail: bool,
        }

        impl MemoryStore {
            fn failing() -> Self {
                Self {
                    fail: true,
                    ..Self::default()
                }
            }
            fn put(&self, key: String, record: &StoredToolApproval) {
                self.values
                    .lock()
                    .unwrap()
                    .insert(key, serde_json::to_string(record).unwrap());
            }
            fn has(&self, key: &str) -> bool {
                self.values.lock().unwrap().contains_key(key)
            }
            fn check(&self) -> StoreResult<()> {
                if self.fail {
                    Err(everruns_provider::error::AgentLoopError::Internal(
                        anyhow::anyhow!("storage down"),
                    ))
                } else {
                    Ok(())
                }
            }
        }

        #[async_trait]
        impl SessionStorageStore for MemoryStore {
            async fn set_value(&self, _: SessionId, key: &str, value: &str) -> StoreResult<()> {
                self.check()?;
                self.values
                    .lock()
                    .unwrap()
                    .insert(key.to_string(), value.to_string());
                Ok(())
            }
            async fn get_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
                self.check()?;
                Ok(self.values.lock().unwrap().get(key).cloned())
            }
            async fn take_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
                self.check()?;
                Ok(self.values.lock().unwrap().remove(key))
            }
            async fn delete_value(&self, _: SessionId, key: &str) -> StoreResult<bool> {
                self.check()?;
                Ok(self.values.lock().unwrap().remove(key).is_some())
            }
            async fn list_keys(&self, _: SessionId) -> StoreResult<Vec<KeyInfo>> {
                Ok(vec![])
            }
            async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> StoreResult<()> {
                Ok(())
            }
            async fn get_secret(&self, _: SessionId, _: &str) -> StoreResult<Option<String>> {
                Ok(None)
            }
            async fn delete_secret(&self, _: SessionId, _: &str) -> StoreResult<bool> {
                Ok(false)
            }
            async fn list_secrets(&self, _: SessionId) -> StoreResult<Vec<SecretInfo>> {
                Ok(vec![])
            }
        }

        fn open_world_tool() -> ToolDefinition {
            tool_with(ToolHints {
                open_world: Some(true),
                ..Default::default()
            })
        }

        fn send(to: &str) -> ToolCall {
            ToolCall {
                id: format!("call_{to}"),
                name: "t".to_string(),
                arguments: json!({ "to": to, "body": "hi" }),
            }
        }

        /// A fresh gate, as a newly started worker process would build it.
        fn fresh_hook(mode: ApprovalMode) -> Arc<dyn PreToolUseHook> {
            ToolApprovalCapability::new(Arc::new(DurableToolApprover)).hook(mode)
        }

        fn context(session_id: SessionId, store: &Arc<MemoryStore>) -> ToolContext {
            ToolContext::new(session_id).with_storage_store_arc(store.clone())
        }

        fn deferred_request(decision: PreToolUseDecision) -> ToolApprovalRequired {
            match decision {
                PreToolUseDecision::Defer { result, .. } => {
                    assert!(result.error.is_some(), "the model must read a failure");
                    ToolApprovalRequired::from_tool_result(&result).expect("structured payload")
                }
                other => panic!("expected the call to be deferred, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn an_open_world_call_is_deferred_until_a_person_answers() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            let request = deferred_request(decision);
            assert_eq!(request.tool, "t");
            assert_eq!(request.tool_call_id, "call_a");
            assert_eq!(request.risk, "open_world");
            assert_eq!(request.mode, "normal");
            assert_eq!(request.fingerprint, approval_fingerprint(&send("a")));
            let asked = DateTime::parse_from_rfc3339(&request.asked_at).unwrap();
            let expires = DateTime::parse_from_rfc3339(&request.expires_at).unwrap();
            assert_eq!(
                (expires - asked).num_seconds(),
                DEFAULT_APPROVAL_TIMEOUT_SECONDS as i64
            );

            // Asked again with nothing recorded — a replayed act — it defers
            // again rather than letting the call through.
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            deferred_request(decision);
        }

        #[tokio::test]
        async fn a_one_off_approval_survives_a_restart_and_is_used_once() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let first = fresh_hook(ApprovalMode::Normal);
            let request = deferred_request(
                first
                    .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                    .await,
            );

            // The API records the answer; the worker that asked is gone.
            drop(first);
            store.put(
                one_off_decision_storage_key(&request.fingerprint),
                &StoredToolApproval::one_off("t", &request.fingerprint, true, Utc::now()),
            );

            // A different process runs the retried call and finds the answer.
            let resumed = fresh_hook(ApprovalMode::Normal);
            let decision = resumed
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            assert!(matches!(decision, PreToolUseDecision::Continue(_)));

            // Consumed: the same call again needs a fresh approval.
            let decision = resumed
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            deferred_request(decision);
        }

        #[tokio::test]
        async fn a_one_off_approval_covers_only_the_exact_call() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let fingerprint = approval_fingerprint(&send("a"));
            store.put(
                one_off_decision_storage_key(&fingerprint),
                &StoredToolApproval::one_off("t", &fingerprint, true, Utc::now()),
            );

            // Different arguments: not what the person saw.
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("b"), &open_world_tool(), &context(session, &store))
                .await;
            deferred_request(decision);
            assert!(
                store.has(&one_off_decision_storage_key(&fingerprint)),
                "a mismatched call must not consume the approval"
            );
        }

        #[tokio::test]
        async fn an_expired_one_off_approval_does_not_let_the_call_through() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let fingerprint = approval_fingerprint(&send("a"));
            let stale = Utc::now() - chrono::Duration::seconds(ONE_OFF_DECISION_TTL_SECONDS + 1);
            store.put(
                one_off_decision_storage_key(&fingerprint),
                &StoredToolApproval::one_off("t", &fingerprint, true, stale),
            );
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            deferred_request(decision);
        }

        #[tokio::test]
        async fn a_one_off_rejection_blocks_the_identical_retry() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let fingerprint = approval_fingerprint(&send("a"));
            store.put(
                one_off_decision_storage_key(&fingerprint),
                &StoredToolApproval::one_off("t", &fingerprint, false, Utc::now()),
            );
            match fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await
            {
                PreToolUseDecision::Block { reason, .. } => assert_eq!(reason, "rejected by user"),
                other => panic!("expected a block, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn always_answers_are_remembered_per_session_and_tool() {
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            store.put(
                always_decision_storage_key("t"),
                &StoredToolApproval::always("t", true, Utc::now()),
            );
            for to in ["a", "b", "c"] {
                let decision = fresh_hook(ApprovalMode::Normal)
                    .before_exec(send(to), &open_world_tool(), &context(session, &store))
                    .await;
                assert!(matches!(decision, PreToolUseDecision::Continue(_)));
            }
            // Never consumed, unlike a one-off answer.
            assert!(store.has(&always_decision_storage_key("t")));

            // A rule for another tool, or a record whose name does not match
            // its key, does not apply.
            let other = Arc::new(MemoryStore::default());
            other.put(
                always_decision_storage_key("t"),
                &StoredToolApproval::always("t2", true, Utc::now()),
            );
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &other))
                .await;
            deferred_request(decision);

            let rejecting = Arc::new(MemoryStore::default());
            rejecting.put(
                always_decision_storage_key("t"),
                &StoredToolApproval::always("t", false, Utc::now()),
            );
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &rejecting))
                .await;
            assert!(matches!(decision, PreToolUseDecision::Block { .. }));
        }

        #[tokio::test]
        async fn an_unreachable_store_blocks_rather_than_allows() {
            let session = SessionId::new_random();
            // No storage at all.
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &ToolContext::new(session))
                .await;
            match decision {
                PreToolUseDecision::Block { reason, .. } => {
                    assert_eq!(reason, "approval unavailable")
                }
                other => panic!("expected a block, got {other:?}"),
            }

            // Storage that errors.
            let store = Arc::new(MemoryStore::failing());
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context(session, &store))
                .await;
            assert!(matches!(decision, PreToolUseDecision::Block { .. }));

            // Asked without a context at all.
            assert_eq!(
                DurableToolApprover
                    .approve(session, &send("a"), &open_world_tool())
                    .await,
                ApprovalDecision::Unavailable
            );
        }

        #[tokio::test]
        async fn a_cancelled_call_is_not_asked_about() {
            let store = Arc::new(MemoryStore::default());
            let token = tokio_util::sync::CancellationToken::new();
            token.cancel();
            let context = context(SessionId::new_random(), &store).with_cancellation(token);
            match fresh_hook(ApprovalMode::Normal)
                .before_exec(send("a"), &open_world_tool(), &context)
                .await
            {
                PreToolUseDecision::Block { reason, .. } => assert_eq!(reason, "turn cancelled"),
                other => panic!("expected a block, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn reads_and_ungated_tools_never_touch_the_store() {
            // Normal mode lets a mutating, non-outward tool through without a
            // lookup, so a broken store cannot stall ordinary work.
            let store = Arc::new(MemoryStore::failing());
            let decision = fresh_hook(ApprovalMode::Normal)
                .before_exec(
                    send("a"),
                    &tool_with(ToolHints::default()),
                    &context(SessionId::new_random(), &store),
                )
                .await;
            assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        }

        #[test]
        fn the_fingerprint_binds_every_byte_but_not_key_order() {
            let mut a = send("a");
            let mut b = send("a");
            a.arguments = json!({ "to": "x", "body": "hi" });
            b.arguments = json!({ "body": "hi", "to": "x" });
            assert_eq!(approval_fingerprint(&a), approval_fingerprint(&b));
            b.arguments = json!({ "body": "hi ", "to": "x" });
            assert_ne!(approval_fingerprint(&a), approval_fingerprint(&b));
            // Keys the loop-detection fingerprint ignores still count here.
            b.arguments = json!({ "body": "hi", "to": "x", "output": "/etc/passwd" });
            assert_ne!(approval_fingerprint(&a), approval_fingerprint(&b));
            b.name = "other".to_string();
            b.arguments = a.arguments.clone();
            assert_ne!(approval_fingerprint(&a), approval_fingerprint(&b));
        }

        #[test]
        fn records_live_under_the_reserved_prefix() {
            assert!(always_decision_storage_key("mcp/x y").starts_with(TOOL_APPROVAL_KV_PREFIX));
            assert!(one_off_decision_storage_key("sha256:ab").starts_with(TOOL_APPROVAL_KV_PREFIX));
            assert_eq!(
                always_decision_storage_key("mcp/x y"),
                "tool_approval/always/mcp_x_y"
            );
        }

        #[test]
        fn large_arguments_are_previewed_not_copied() {
            let big = json!({ "body": "x".repeat(20_000) });
            let (preview, truncated) = preview_arguments(&big);
            assert!(truncated);
            assert!(preview.as_str().unwrap().len() <= ARGUMENTS_PREVIEW_BYTES);
            let small = json!({ "body": "x" });
            assert_eq!(preview_arguments(&small), (small, false));
        }

        #[test]
        fn timeout_is_configurable_and_validated() {
            let capability = ToolApprovalCapability::new(Arc::new(DurableToolApprover));
            assert!(
                capability
                    .validate_config(&json!({"mode": "normal", "timeout_seconds": 120}))
                    .is_ok()
            );
            assert!(
                capability
                    .validate_config(&json!({"timeout_seconds": 5}))
                    .is_err()
            );
            assert!(
                capability
                    .validate_config(&json!({"timeout_seconds": "soon"}))
                    .is_err()
            );
            assert_eq!(approval_timeout_from_config(&json!({})), 900);
            assert_eq!(
                approval_timeout_from_config(&json!({"timeout_seconds": 1})),
                60
            );
            assert_eq!(
                approval_timeout_from_config(&json!({"timeout_seconds": 120})),
                120
            );
        }
    }
}
