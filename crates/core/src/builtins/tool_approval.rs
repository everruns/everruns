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
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use everruns_contracts::rt::Instant;
use everruns_contracts::tool_approval_policy::{
    AUTO_REVIEW_STREAK_KEY, AutoReviewStreak, SESSION_APPROVAL_MODE_KEY, SessionToolApprovalPolicy,
    ToolApprovalRule, ToolRuleAction, strictest_rule,
};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::builtins::capabilities::{Capability, CapabilityStatus};
use crate::builtins::tool_hooks::{PolicyGate, PreToolUseDecision, PreToolUseHook};
use crate::builtins::tool_types::{
    TOOL_APPROVAL_REQUIRED_CODE, ToolApprovalRequired, ToolCall, ToolDefinition, ToolPolicy,
    ToolResult, preview_tool_arguments,
};

use crate::builtins::events::{EventRequest, ToolReviewedData};
pub use crate::builtins::tool_approval_durable::{
    DurableToolApprover, TOOL_APPROVAL_KV_PREFIX, approval_fingerprint,
};
use crate::builtins::tool_review::{ReviewVerdict, ToolReviewer, build_review_request};
use crate::builtins::typed_id::SessionId;
use crate::tool_context::ToolContext;

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

    /// What [`approve_in_context`](Self::approve_in_context) would answer,
    /// read without asking anyone or using up an answer. `None` when that
    /// cannot be known without asking, which is the default: an interactive
    /// approver only knows by prompting.
    async fn peek_in_context(
        &self,
        _session_id: SessionId,
        _tool_call: &ToolCall,
        _context: &ToolContext,
    ) -> Option<ApprovalDecision> {
        None
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
    /// Every state-changing call goes to a reviewer model first, which lets
    /// it run, blocks it with a reason, or hands it to a person. Without a
    /// reviewer this asks a person exactly like `protective`.
    Auto,
    /// Never pause for approval; act autonomously.
    Off,
}

impl ApprovalMode {
    /// Canonical lowercase name, as it appears in capability config.
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalMode::Protective => "protective",
            ApprovalMode::Normal => "normal",
            ApprovalMode::Auto => "auto",
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
            "auto" | "auto-review" | "auto_review" | "autoreview" | "reviewed" => {
                Some(ApprovalMode::Auto)
            }
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
/// [`ToolHints`]: crate::builtins::tool_types::ToolHints
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
        // Auto reviews everything Protective would ask about; the reviewer,
        // not the threshold, is what keeps it from asking a person.
        ApprovalMode::Auto | ApprovalMode::Protective => !matches!(risk, ToolRisk::ReadOnly),
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
    reviewer: Option<Arc<dyn ToolReviewer>>,
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
            reviewer: None,
            remembered: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Answer gated calls with `reviewer` first when the mode is
    /// [`ApprovalMode::Auto`].
    ///
    /// The reviewer lets a call run, blocks it with a reason the agent reads,
    /// or hands it to the approver. Without one, `auto` asks the approver about
    /// everything `protective` would.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use everruns_core::builtins::{
    ///     DurableToolApprover, ModelToolReviewer, ToolApprovalCapability,
    /// };
    ///
    /// let gate = ToolApprovalCapability::new(Arc::new(DurableToolApprover))
    ///     .with_reviewer(Arc::new(ModelToolReviewer));
    /// # let _ = gate;
    /// ```
    pub fn with_reviewer(mut self, reviewer: Arc<dyn ToolReviewer>) -> Self {
        self.reviewer = Some(reviewer);
        self
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

    pub(crate) fn hook(&self, mode: ApprovalMode) -> Arc<dyn PreToolUseHook> {
        self.hook_with(GateConfig {
            mode,
            ..GateConfig::default()
        })
    }

    fn hook_with(&self, config: GateConfig) -> Arc<dyn PreToolUseHook> {
        // A policy gate, so it decides on the call after every argument
        // rewrite: what a person approves is what runs (EVE-1184).
        Arc::new(PolicyGate(ToolApprovalHook {
            approver: self.approver.clone(),
            reviewer: self.reviewer.clone(),
            config,
            policy: self.policy.clone(),
            remembered: self.remembered.clone(),
        }))
    }
}

/// The agent's own settings for the gate, read from its capability config.
#[derive(Debug, Clone)]
struct GateConfig {
    mode: ApprovalMode,
    /// Window a deferred request stays open before it counts as rejected.
    timeout_seconds: u64,
    /// The agent's rules.
    rules: Vec<ToolApprovalRule>,
    /// Notes for the auto reviewer, from the agent's manager.
    notes: Vec<String>,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::default(),
            timeout_seconds: DEFAULT_APPROVAL_TIMEOUT_SECONDS,
            rules: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl GateConfig {
    fn from_config(config: &serde_json::Value) -> Self {
        Self {
            mode: ApprovalMode::from_config(config),
            timeout_seconds: approval_timeout_from_config(config),
            rules: config
                .get("rules")
                .cloned()
                .and_then(|rules| serde_json::from_value(rules).ok())
                .unwrap_or_default(),
            notes: config
                .pointer("/auto/notes")
                .cloned()
                .and_then(|notes| serde_json::from_value(notes).ok())
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Capability for ToolApprovalCapability {
    fn localizations(&self) -> Vec<crate::capabilities::CapabilityLocalization> {
        vec![crate::capabilities::CapabilityLocalization::text(
            "uk",
            "Підтвердження інструментів",
            "Призупиняє ризиковані виклики інструментів до інтерактивного підтвердження відповідно до вибраного режиму.",
        )]
    }

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
                    "enum": ["off", "auto", "normal", "protective"],
                    "default": "normal",
                    "title": "Approval mode",
                    "description": "off: never ask. auto: a reviewer model checks every call that is not read-only, lets routine work run, blocks risky calls with a reason, and asks a person when unsure. normal: ask before tools that declare themselves destructive or outward-facing. protective: ask before anything that is not declared read-only.",
                },
                "rules": {
                    "type": "array",
                    "maxItems": everruns_contracts::tool_approval_policy::MAX_APPROVAL_RULES,
                    "title": "Rules",
                    "description": "Per-tool rules that apply in every mode. tool is an exact name or a pattern with *; action allow runs without approval, ask always asks a person, never blocks. The most cautious matching rule wins.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": {"type": "string", "minLength": 1, "maxLength": everruns_contracts::tool_approval_policy::MAX_RULE_PATTERN_LEN},
                            "action": {"type": "string", "enum": ["allow", "ask", "never"]}
                        },
                        "required": ["tool", "action"],
                        "additionalProperties": false
                    }
                },
                "auto": {
                    "type": "object",
                    "title": "Auto review",
                    "properties": {
                        "notes": {
                            "type": "array",
                            "items": {"type": "string"},
                            "maxItems": everruns_contracts::tool_approval_policy::MAX_TRUST_NOTES,
                            "title": "Notes for the reviewer",
                            "description": "Plain sentences the auto reviewer reads, such as trusted domains or which systems count as production."
                        }
                    },
                    "additionalProperties": false
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
                if matches!(mode.as_str(), "off" | "auto" | "normal" | "protective") => {}
            Some(other) => {
                return Err(format!(
                    "tool_approval mode must be one of off|auto|normal|protective, got {other}"
                ));
            }
        }
        if let Some(rules) = object.get("rules") {
            let rules: Vec<ToolApprovalRule> = serde_json::from_value(rules.clone())
                .map_err(|error| format!("tool_approval rules are invalid: {error}"))?;
            everruns_contracts::tool_approval_policy::validate_agent_rules(&rules)
                .map_err(|error| format!("tool_approval {error}"))?;
        }
        if let Some(auto) = object.get("auto") {
            let Some(auto) = auto.as_object() else {
                return Err("tool_approval auto must be an object".to_string());
            };
            if let Some(key) = auto.keys().find(|key| key.as_str() != "notes") {
                return Err(format!("tool_approval auto has an unknown field '{key}'"));
            }
            if let Some(notes) = auto.get("notes") {
                let notes: Vec<String> = serde_json::from_value(notes.clone())
                    .map_err(|_| "tool_approval auto.notes must be a list of text".to_string())?;
                everruns_contracts::tool_approval_policy::validate_reviewer_notes(&notes)
                    .map_err(|error| format!("tool_approval {error}"))?;
            }
        }
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "mode" | "timeout_seconds" | "rules" | "auto"))
        {
            return Err(format!("tool_approval has an unknown field '{key}'"));
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
        vec![self.hook_with(GateConfig::from_config(config))]
    }
}

struct ToolApprovalHook {
    approver: Arc<dyn ToolApprover>,
    reviewer: Option<Arc<dyn ToolReviewer>>,
    config: GateConfig,
    policy: Option<ToolApprovalPolicy>,
    /// "Allow always" / "reject always" answers, keyed by (session, tool name).
    /// `true` = remembered allow, `false` = remembered reject.
    remembered: Arc<Mutex<HashMap<(SessionId, String), bool>>>,
}

/// How long a worker reuses a session's organization and personal policy.
const SESSION_POLICY_TTL: Duration = Duration::from_secs(60);
/// Sessions whose policy one process remembers before starting over.
const SESSION_POLICY_CACHE_LIMIT: usize = 4_096;

type CachedPolicy = (Instant, Option<Arc<SessionToolApprovalPolicy>>);
static SESSION_POLICIES: LazyLock<Mutex<HashMap<SessionId, CachedPolicy>>> =
    LazyLock::new(Default::default);

/// The session's organization policy and owner's settings, from the host.
/// `None` when the host has none or the lookup failed: the gate then follows
/// the agent's config alone, which is never looser than with a policy, except
/// for org `allow` rules, which simply do not apply.
async fn session_policy(context: &ToolContext) -> Option<Arc<SessionToolApprovalPolicy>> {
    let resolver = context.connection_resolver.as_ref()?;
    if let Some((at, cached)) = SESSION_POLICIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&context.session_id)
        && at.elapsed() < SESSION_POLICY_TTL
    {
        return cached.clone();
    }
    let fetched = match resolver.get_tool_approval_policy(context.session_id).await {
        Ok(policy) => policy.map(Arc::new),
        Err(error) => {
            tracing::warn!(
                session_id = %context.session_id,
                %error,
                "tool approval policy unavailable; using the agent's config alone"
            );
            return None;
        }
    };
    let mut cache = SESSION_POLICIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if cache.len() >= SESSION_POLICY_CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(context.session_id, (Instant::now(), fetched.clone()));
    fetched
}

/// A person's mode switch for this session, if they made one.
async fn session_mode_switch(context: &ToolContext) -> Option<ApprovalMode> {
    let store = context.storage_store.as_ref()?;
    match store
        .get_value(context.session_id, SESSION_APPROVAL_MODE_KEY)
        .await
    {
        Ok(value) => value.as_deref().and_then(ApprovalMode::parse),
        Err(error) => {
            tracing::warn!(session_id = %context.session_id, %error, "could not read the session's approval mode");
            None
        }
    }
}

/// The mode for this call: the person's session switch, else their own
/// default, else the agent's; held to what the organization allows.
///
/// A person may not switch off a gate the agent's manager turned on: when the
/// agent's mode is not `off`, the loosest a person can pick is `auto`, which
/// still has a reviewer.
fn resolve_mode(
    agent: ApprovalMode,
    switched: Option<ApprovalMode>,
    policy: Option<&SessionToolApprovalPolicy>,
) -> ApprovalMode {
    let personal = policy
        .and_then(|p| p.personal.default_mode.as_deref())
        .and_then(ApprovalMode::parse);
    let mut mode = switched.or(personal).unwrap_or(agent);
    if mode == ApprovalMode::Off && agent != ApprovalMode::Off {
        mode = ApprovalMode::Auto;
    }
    match policy {
        Some(policy) => ApprovalMode::parse(policy.org.cap(mode.as_str())).unwrap_or(mode),
        None => mode,
    }
}

async fn load_streak(context: &ToolContext) -> AutoReviewStreak {
    let Some(store) = context.storage_store.as_ref() else {
        return AutoReviewStreak::default();
    };
    store
        .get_value(context.session_id, AUTO_REVIEW_STREAK_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

async fn save_streak(context: &ToolContext, streak: AutoReviewStreak) {
    let Some(store) = context.storage_store.as_ref() else {
        return;
    };
    let Ok(raw) = serde_json::to_string(&streak) else {
        return;
    };
    if let Err(error) = store
        .set_value(context.session_id, AUTO_REVIEW_STREAK_KEY, &raw)
        .await
    {
        tracing::warn!(session_id = %context.session_id, %error, "could not record the auto review streak");
    }
}

/// What the gate does with a call before asking anyone.
enum Route {
    Run,
    Block(&'static str),
    /// Ask; `person_only` means a reviewer may not answer.
    Ask {
        person_only: bool,
    },
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

    /// Rules first, then the mode's threshold.
    fn route(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        mode: ApprovalMode,
        policy: Option<&SessionToolApprovalPolicy>,
    ) -> Route {
        let org_rules = policy.map(|p| p.org.rules.as_slice()).unwrap_or_default();
        let personal_rules = policy
            .map(|p| p.personal.rules.as_slice())
            .unwrap_or_default();
        // Personal rules only ever ask or block: an `allow` that slipped into
        // storage is ignored here as well as refused on write.
        let rule = strictest_rule(
            self.config.rules.iter().chain(org_rules).chain(
                personal_rules
                    .iter()
                    .filter(|r| r.action != ToolRuleAction::Allow),
            ),
            &tool_call.name,
        );
        match rule {
            Some(ToolRuleAction::Never) => return Route::Block("blocked by an approval rule"),
            Some(ToolRuleAction::Ask) => return Route::Ask { person_only: true },
            Some(ToolRuleAction::Allow) => return Route::Run,
            None => {}
        }
        let gated = match &self.policy {
            Some(policy) => policy(tool_call, tool_def),
            None => requires_approval(mode, classify(tool_def)),
        };
        if !gated {
            return Route::Run;
        }
        // A host policy's predicate, or a definition that itself requires
        // approval, is a promise that a person decides.
        let person_only =
            self.policy.is_some() || matches!(tool_def.policy(), ToolPolicy::RequiresApproval);
        Route::Ask { person_only }
    }

    /// The mode for this call. Read-only calls with no rule never need it, so
    /// callers skip the storage read for them.
    async fn mode(
        &self,
        context: &ToolContext,
        policy: Option<&SessionToolApprovalPolicy>,
    ) -> ApprovalMode {
        resolve_mode(self.config.mode, session_mode_switch(context).await, policy)
    }

    /// Park the call on a durable approval request instead of running it.
    fn defer(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        mode: ApprovalMode,
        review_note: Option<String>,
    ) -> PreToolUseDecision {
        let asked_at = Utc::now();
        let window = i64::try_from(self.config.timeout_seconds)
            .unwrap_or(MAX_APPROVAL_TIMEOUT_SECONDS as i64)
            .clamp(
                MIN_APPROVAL_TIMEOUT_SECONDS as i64,
                MAX_APPROVAL_TIMEOUT_SECONDS as i64,
            );
        let expires_at = asked_at + chrono::Duration::seconds(window);
        let (arguments, arguments_truncated) = preview_tool_arguments(&tool_call.arguments);
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
            mode: mode.as_str().to_string(),
            asked_at: asked_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
            review_note,
        };
        let result = ToolResult {
            details: None,
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::to_value(&request).unwrap_or_default()),
            images: None,
            error: Some(error),
            connection_required: None,
            raw_output: None,
        };
        PreToolUseDecision::Defer { tool_call, result }
    }

    /// Ask the host approver, as every mode but `auto` always has.
    async fn ask_person(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
        mode: ApprovalMode,
        review_note: Option<String>,
    ) -> PreToolUseDecision {
        let key = (context.session_id, tool_call.name.clone());
        let decision = self
            .approver
            .approve_in_context(context.session_id, &tool_call, tool_def, context)
            .await;
        if mode == ApprovalMode::Auto
            && matches!(
                decision,
                ApprovalDecision::Allow | ApprovalDecision::AllowAlways
            )
        {
            // A person's yes resumes auto review after a run of blocks.
            save_streak(context, AutoReviewStreak::default()).await;
        }
        match decision {
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
            ApprovalDecision::Deferred => self.defer(tool_call, tool_def, mode, review_note),
        }
    }

    async fn emit_review(
        &self,
        context: &ToolContext,
        tool_call: &ToolCall,
        verdict: &str,
        stage: &str,
        rule: Option<String>,
        reason: &str,
    ) {
        tracing::info!(
            target: "tool_approval.auto_review",
            session_id = %context.session_id,
            tool = %tool_call.name,
            verdict,
            stage,
            rule = rule.as_deref().unwrap_or(""),
            "auto review"
        );
        let Some(emitter) = context.event_emitter.as_ref() else {
            return;
        };
        let event_context = context.event_context.clone().unwrap_or_default();
        let data = ToolReviewedData {
            turn_id: event_context.turn_id,
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            verdict: verdict.to_string(),
            stage: stage.to_string(),
            rule,
            reason: reason.to_string(),
            fingerprint: approval_fingerprint(tool_call),
        };
        if let Err(error) = emitter
            .emit(EventRequest::new(context.session_id, event_context, data))
            .await
        {
            tracing::warn!(session_id = %context.session_id, %error, "failed to emit tool.reviewed");
        }
    }

    /// `auto`: let the reviewer answer before a person is asked.
    async fn auto_review(
        &self,
        reviewer: &dyn ToolReviewer,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
        policy: Option<&SessionToolApprovalPolicy>,
    ) -> PreToolUseDecision {
        let mode = ApprovalMode::Auto;
        // A person already answered this call or tool: their answer stands.
        if self
            .approver
            .peek_in_context(context.session_id, &tool_call, context)
            .await
            .is_some_and(|decision| decision != ApprovalDecision::Deferred)
        {
            return self
                .ask_person(tool_call, tool_def, context, mode, None)
                .await;
        }
        let mut streak = load_streak(context).await;
        if streak.paused() {
            let reason = "Auto review blocked several calls in a row, so a person decides for now";
            self.emit_review(context, &tool_call, "asked_person", "paused", None, reason)
                .await;
            return self
                .ask_person(tool_call, tool_def, context, mode, Some(reason.to_string()))
                .await;
        }
        let mut notes = policy
            .map(|p| p.org.trust_notes.clone())
            .unwrap_or_default();
        notes.extend(self.config.notes.iter().cloned());
        let request = build_review_request(&tool_call, risk_label(tool_def), notes, context).await;
        match reviewer.review(&request, context).await {
            ReviewVerdict::Allow { stage, reason } => {
                if streak.consecutive > 0 {
                    streak.consecutive = 0;
                    save_streak(context, streak).await;
                }
                self.emit_review(context, &tool_call, "allowed", stage, None, &reason)
                    .await;
                PreToolUseDecision::Continue(tool_call)
            }
            ReviewVerdict::Block {
                stage,
                rule,
                reason,
            } => {
                streak.consecutive = streak.consecutive.saturating_add(1);
                streak.total = streak.total.saturating_add(1);
                save_streak(context, streak).await;
                self.emit_review(context, &tool_call, "blocked", stage, rule.clone(), &reason)
                    .await;
                let label = rule.map(|r| format!(" [{r}]")).unwrap_or_default();
                let message = format!(
                    "Auto review blocked `{name}`{label}: {reason}. It did not run. Do not repeat \
                     the same call unchanged. Reach the goal a safer way, or, if this exact action \
                     is needed, ask the person in the conversation, saying what you want to do \
                     and why; if they agree, call it again.",
                    name = tool_call.name,
                );
                PreToolUseDecision::Block {
                    user_message: Some(format!(
                        "Auto review blocked `{}`{label}: {reason}",
                        tool_call.name
                    )),
                    reason: message,
                    tool_call,
                }
            }
            ReviewVerdict::AskPerson { stage, reason } => {
                self.emit_review(context, &tool_call, "asked_person", stage, None, &reason)
                    .await;
                self.ask_person(tool_call, tool_def, context, mode, Some(reason))
                    .await
            }
            ReviewVerdict::Unavailable { reason } => {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool = %tool_call.name,
                    reason,
                    "auto review gave no verdict; asking a person"
                );
                let note = "Auto review could not decide, so a person decides";
                self.emit_review(
                    context,
                    &tool_call,
                    "asked_person",
                    "unavailable",
                    None,
                    note,
                )
                .await;
                self.ask_person(tool_call, tool_def, context, mode, Some(note.to_string()))
                    .await
            }
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
        let policy = session_policy(context).await;
        let policy = policy.as_deref();
        // Read-only tools with no rule never wait, in any mode: skip the
        // storage read the mode needs.
        let mode = if self.policy.is_none() && classify(tool_def) == ToolRisk::ReadOnly {
            self.config.mode
        } else {
            self.mode(context, policy).await
        };
        let person_only = match self.route(&tool_call, tool_def, mode, policy) {
            Route::Run => return PreToolUseDecision::Continue(tool_call),
            Route::Block(reason) => return Self::block(tool_call, reason),
            Route::Ask { person_only } => person_only,
        };

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

        if mode == ApprovalMode::Auto
            && !person_only
            && let Some(reviewer) = self.reviewer.clone()
        {
            return self
                .auto_review(reviewer.as_ref(), tool_call, tool_def, context, policy)
                .await;
        }
        self.ask_person(tool_call, tool_def, context, mode, None)
            .await
    }

    /// Only a call that would be parked on a request is reported: the request
    /// is raised as if the call had been made, and nothing else changes. In
    /// `auto` that depends on a reviewer's verdict, which a preview does not
    /// ask for, so nothing is reported.
    async fn preview(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Option<ToolResult> {
        let policy = session_policy(context).await;
        let policy = policy.as_deref();
        let mode = self.mode(context, policy).await;
        let person_only = match self.route(tool_call, tool_def, mode, policy) {
            Route::Ask { person_only } => person_only,
            Route::Run | Route::Block(_) => return None,
        };
        if mode == ApprovalMode::Auto && !person_only && self.reviewer.is_some() {
            return None;
        }
        let key = (context.session_id, tool_call.name.clone());
        let remembered = self
            .remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&key);
        if remembered {
            return None;
        }
        let decision = self
            .approver
            .peek_in_context(context.session_id, tool_call, context)
            .await;
        match (
            decision,
            self.defer(tool_call.clone(), tool_def, mode, None),
        ) {
            (Some(ApprovalDecision::Deferred), PreToolUseDecision::Defer { result, .. }) => {
                Some(result)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::tool_types::{BuiltinTool, ToolHints};
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
}

#[cfg(test)]
#[path = "tool_approval_auto_tests.rs"]
mod auto_tests;
