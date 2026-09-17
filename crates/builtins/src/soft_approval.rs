// Soft approval — spoken-consent confirmation for critical actions.
//
// This is prompt-engineering, not a permission gate. A hard, per-call yes/no
// gate (`tool_approval`) interrupts safe work, cannot be reasoned about, and
// trains people to approve reflexively. Soft approval takes the opposite tack:
// the agent is told, in its system prompt, to batch the safe steps and to stop
// only in front of the genuinely critical ones, state what is at risk, and ask
// one short question. The model decides what is critical, the user approves in
// plain language, and the grant is written to the session's event log.
//
// The pause is a tool call, not prose. A model that ends its turn on
// "Squash-merging." has stopped, but the loop, the transcript, and the user all
// see what they would see if it had finished: text, then nothing. So pausing
// means calling `request_approval` and ending the turn there. The call is the
// pause: it gives hosts a fact to render ([`PendingApprovalStore`]) and the
// audit log a record of what was *asked*, not only of what was granted.
//
// Composition. `tool_approval` is the hard layer and can actually stop a call;
// this layer cannot, it only asks the model to. Both read the same
// [`ApprovalMode`] vocabulary, so one level configures both consistently. The
// three tools here declare themselves read-only precisely so the hard gate
// never asks a human to approve the act of asking a human.
//
// Portability. Everything here is backend-neutral: no database, no bespoke
// approval UI, no host services. State is per-session and in-memory by
// default, and a host that owns its own level (a CLI with a settings file) can
// supply an [`ApprovalModeStore`] instead. That is the seam that lets a
// terminal host migrate onto this capability without giving up its central,
// cross-session setting.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, arg_str, labeled_phrase, truncate,
};
use crate::tool_types::{DeferrablePolicy, ToolCall, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use everruns_core::tool_context::ToolContext;

pub use crate::tool_approval::ApprovalMode;

pub const SOFT_APPROVAL_CAPABILITY_ID: &str = "soft_approval";

/// A critical action the agent has stopped in front of, waiting for a yes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingApproval {
    /// What the agent will do once approved.
    pub action: String,
    /// The question put to the user, already phrased for display.
    pub question: String,
}

/// The one pending approval a session can be holding.
///
/// `request_approval` sets it, `record_approval` clears it, and a host reads it
/// when a turn ends so a pause is visible rather than looking like a turn that
/// died. A pause outlives the turn that raised it: it is answered by the user's
/// next message, not by the turn ending, so hosts call [`resolve`] when that
/// message arrives.
///
/// [`resolve`]: PendingApprovalStore::resolve
#[derive(Clone, Default)]
pub struct PendingApprovalStore {
    inner: Arc<Mutex<HashMap<SessionId, PendingApproval>>>,
}

impl PendingApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Raise a pause. `request_approval` owns this in production; hosts use it
    /// in tests to stand in for a model that paused.
    pub fn set(&self, session_id: SessionId, pending: PendingApproval) {
        self.lock().insert(session_id, pending);
    }

    /// What the session is waiting on, without consuming it.
    pub fn peek(&self, session_id: &SessionId) -> Option<PendingApproval> {
        self.lock().get(session_id).cloned()
    }

    /// Drop a pause the user has now answered, whichever way they answered.
    pub fn resolve(&self, session_id: &SessionId) {
        self.lock().remove(session_id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, PendingApproval>> {
        // A poisoned lock holds a plain map with no broken invariant; losing
        // every session's pause to an unrelated panic would be worse.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Where the effective approval level is read from and written to.
///
/// The default ([`SessionApprovalModes`]) is a per-session, in-memory override
/// over the capability config. A host with its own durable, cross-session
/// setting implements this instead, so `set_approval_mode` writes through to
/// that setting and the level survives a restart.
pub trait ApprovalModeStore: Send + Sync {
    /// The level in force for this session, or `None` to fall back to the
    /// capability config.
    fn mode(&self, session_id: &SessionId) -> Option<ApprovalMode>;

    /// Record a new level. `Err` is reported to the model verbatim.
    fn set_mode(&self, session_id: &SessionId, mode: ApprovalMode) -> Result<(), String>;
}

/// Default [`ApprovalModeStore`]: an in-memory override per session.
///
/// Deliberately transient. The durable level is the agent's capability config;
/// this only holds "be more careful for the rest of this conversation", which
/// is exactly as long-lived as the conversation it was said in.
#[derive(Clone, Default)]
pub struct SessionApprovalModes {
    inner: Arc<Mutex<HashMap<SessionId, ApprovalMode>>>,
}

impl SessionApprovalModes {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ApprovalModeStore for SessionApprovalModes {
    fn mode(&self, session_id: &SessionId) -> Option<ApprovalMode> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .copied()
    }

    fn set_mode(&self, session_id: &SessionId, mode: ApprovalMode) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(*session_id, mode);
        Ok(())
    }
}

/// Render the `<soft_approval>` system-prompt block for a level.
///
/// Pure, so the per-level branch is testable without a `SystemPromptContext`.
/// Returns `None` for [`ApprovalMode::Off`], which contributes nothing.
pub fn render_approval_block(mode: ApprovalMode) -> Option<String> {
    let threshold = match mode {
        ApprovalMode::Off => return None,
        ApprovalMode::Protective => {
            "PROTECTIVE — the bar is low. Ask before ANY action that changes \
             state: writing or deleting files, commits and pushes, installing \
             or removing packages, requests with side effects, creating or \
             modifying shared resources, or running a command that is not \
             plainly read-only."
        }
        ApprovalMode::Normal => {
            "NORMAL — ask only before clearly DANGEROUS actions: destructive or \
             irreversible operations (deleting files or data, dropping records, \
             force-push, history rewrites) and outward-facing ones (pushing, \
             publishing, opening pull requests, sending messages or mail, \
             deploying, spending money). Ordinary edits and local, reversible \
             changes proceed without asking."
        }
    };

    Some(format!(
        "<soft_approval>\n\
Soft approval is active at level {level}.\n\
\n\
{threshold}\n\
\n\
How to operate:\n\
- Plan, then BATCH the safe steps and run them without pausing. Do NOT ask \
before every tool call; read-only inspection never needs approval.\n\
- At a critical action, STOP: say briefly what you will do, why, and what is \
at risk, then call `request_approval` with that action and one short question, \
and end your turn. That call IS the pause, and the only signal to the user \
that you are waiting.\n\
- NEVER announce a critical action and then stop without it. \"Deploying now.\" \
as your last words reads as a turn that died, not as a question. Ask, or act; \
never narrate and halt.\n\
- A plain affirmative (\"yes\", \"go ahead\", \"do it\") is the approval; a \
negative or hesitant reply is not. Consent is spoken in the conversation.\n\
- Immediately after they approve, call `record_approval` with what was \
approved, then carry it out. This writes it to the session audit log.\n\
- One approval covers that action, not later ones. Honor a pre-authorized \
category (\"no need to ask for commits\") without re-asking, and a workflow the \
user invoked by name pre-authorizes the actions it exists to perform: run \
those without pausing and note the grant once with `record_approval`.\n\
- If the user asks you to be more or less cautious, call `set_approval_mode`.\n\
</soft_approval>",
        level = mode.as_str(),
    ))
}

/// Spoken-consent approval for critical actions.
pub struct SoftApprovalCapability {
    pending: PendingApprovalStore,
    modes: Arc<dyn ApprovalModeStore>,
}

impl Default for SoftApprovalCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftApprovalCapability {
    /// Build the capability with the default per-session mode overrides.
    pub fn new() -> Self {
        Self {
            pending: PendingApprovalStore::new(),
            modes: Arc::new(SessionApprovalModes::new()),
        }
    }

    /// Build the capability over a host-owned level store, for hosts whose
    /// approval level is a durable setting of their own.
    pub fn with_mode_store(modes: Arc<dyn ApprovalModeStore>) -> Self {
        Self {
            pending: PendingApprovalStore::new(),
            modes,
        }
    }

    /// The store hosts read to find out whether a turn ended on a pause.
    pub fn pending(&self) -> PendingApprovalStore {
        self.pending.clone()
    }

    /// The level in force: a session or host override first, then the
    /// capability config, then the default.
    pub fn effective_mode(&self, session_id: &SessionId, config: &Value) -> ApprovalMode {
        self.modes
            .mode(session_id)
            .unwrap_or_else(|| ApprovalMode::from_config(config))
    }
}

#[async_trait]
impl Capability for SoftApprovalCapability {
    fn id(&self) -> &str {
        SOFT_APPROVAL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Soft Approval"
    }

    fn description(&self) -> &str {
        "Asks the agent to pause and get spoken consent before destructive or outward-facing \
         actions, batching safe work without interruption. Guidance, not a hard gate."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "М'яке підтвердження",
            "Просить агента зупинитися й отримати усну згоду перед руйнівними або зовнішніми діями, \
             виконуючи безпечні кроки без зупинок. Це настанова, а не жорсткий бар'єр.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn icon(&self) -> Option<&str> {
        Some("shield-question")
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["off", "normal", "protective"],
                    "default": "normal",
                    "title": "Approval level",
                    "description": "off: never pause. normal: pause before destructive or outward-facing actions. protective: pause before anything that changes state."
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("soft_approval config must be an object".to_string());
        }
        match config.get("mode") {
            None => Ok(()),
            Some(Value::String(mode)) if ApprovalMode::parse(mode).is_some() => Ok(()),
            Some(value) => Err(format!(
                "mode must be one of \"off\", \"normal\", \"protective\", got {value}"
            )),
        }
    }

    async fn system_prompt_contribution(
        &self,
        ctx: &crate::capabilities::SystemPromptContext,
    ) -> Option<String> {
        // Callers that collect without per-capability config still get the
        // level in force: a session override, else the default.
        render_approval_block(self.effective_mode(&ctx.session_id, &Value::Null))
    }

    async fn system_prompt_contribution_with_config(
        &self,
        ctx: &crate::capabilities::SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        // Resolved per turn, so `set_approval_mode` and a config edit both take
        // effect on the very next turn without restarting the session.
        render_approval_block(self.effective_mode(&ctx.session_id, config))
    }

    fn system_prompt_preview(&self) -> Option<String> {
        // `off` would contribute nothing; preview the default level.
        render_approval_block(ApprovalMode::Normal)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(RequestApprovalTool {
                pending: self.pending.clone(),
            }),
            Box::new(RecordApprovalTool {
                pending: self.pending.clone(),
            }),
            Box::new(SetApprovalModeTool {
                modes: self.modes.clone(),
            }),
        ]
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// Hints shared by all three tools.
///
/// They are read-only by design: none of them touches the host, they only move
/// a marker and write an audit line. Declaring that keeps the hard
/// `tool_approval` gate from stopping a call whose entire purpose is to ask a
/// human, which at `protective` would deadlock the two layers against each
/// other.
fn approval_tool_hints() -> ToolHints {
    ToolHints::default()
        .with_readonly(true)
        .with_destructive(false)
        .with_open_world(false)
}

const FALLBACK_APPROVAL_ACTION: &str = "the pending critical action approved in the conversation";
const FALLBACK_APPROVAL_QUESTION: &str = "Go ahead?";

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The approved action, tolerating a model that passed a bare string or
/// nothing at all after consent was already spoken.
fn approval_action(arguments: &Value) -> &str {
    if let Some(action) = arguments.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return action;
    }
    non_empty_str(arguments.get("action")).unwrap_or(FALLBACK_APPROVAL_ACTION)
}

/// Pauses in front of a critical action and asks the user to approve it.
struct RequestApprovalTool {
    pending: PendingApprovalStore,
}

#[async_trait]
impl Tool for RequestApprovalTool {
    fn name(&self) -> &str {
        "request_approval"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Ask approval")
    }

    fn description(&self) -> &str {
        // Deliberately terse: this tool is never deferred behind tool search
        // (the model has to find it the instant it decides to pause), so every
        // byte is paid on every turn.
        "Pause before a critical action and ask the user to approve it. Call this INSTEAD of \
         announcing the action and stopping: the call is what tells the user you are waiting. \
         End your turn right after. Logged; no secrets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action awaiting approval, e.g. \"delete the staging database\"."
                },
                "question": {
                    "type": "string",
                    "description": "One short question for the user, e.g. \"This drops 12k rows. Go ahead?\""
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        approval_tool_hints()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let action = arg_str(&tool_call.arguments, &["action"]).map(|value| truncate(value, 48));
        Some(labeled_phrase(
            "Asking approval",
            "Awaiting approval",
            "Approval request failed",
            action,
            phase,
        ))
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Without a session there is nowhere to hang the pause, but the model
        // must still be told to stop, so the marker is skipped and the result
        // is unchanged.
        approval_result(&arguments)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let (action, question) = approval_ask(&arguments);
        self.pending
            .set(context.session_id, PendingApproval { action, question });
        approval_result(&arguments)
    }
}

fn approval_ask(arguments: &Value) -> (String, String) {
    let action = approval_action(arguments).to_string();
    let question = non_empty_str(arguments.get("question"))
        .unwrap_or(FALLBACK_APPROVAL_QUESTION)
        .to_string();
    (action, question)
}

fn approval_result(arguments: &Value) -> ToolExecutionResult {
    let (action, question) = approval_ask(arguments);
    ToolExecutionResult::success(json!({
        "ok": true,
        "awaiting_approval": true,
        "action": action,
        "question": question,
        "message": "waiting for the user to approve; end your turn now",
    }))
}

/// Records that the user gave spoken approval for a critical action.
///
/// The tool only echoes the record back; the durable audit entry is the
/// `tool.completed` event this call produces on the session, which captures the
/// arguments, the output, and the timestamp. No separate audit store.
struct RecordApprovalTool {
    pending: PendingApprovalStore,
}

#[async_trait]
impl Tool for RecordApprovalTool {
    fn name(&self) -> &str {
        "record_approval"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Record approval")
    }

    fn description(&self) -> &str {
        "Record that the user just gave spoken approval for a critical action, for the audit \
         trail. Call this immediately after the user says yes and before carrying the action \
         out. Pass a concise, specific description of exactly what was approved. These arguments \
         are written to the session log, so do NOT include secrets (API keys, tokens, \
         passwords); describe the action and redact any sensitive values."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The specific action the user approved, e.g. \"force-push branch feature/x to origin\". Do not embed secrets."
                },
                "detail": {
                    "type": "string",
                    "description": "Optional extra context: the command, affected resources, or scope of the approval. Redact any secrets before passing them — this is logged."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        approval_tool_hints()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let action = arg_str(&tool_call.arguments, &["action"]).map(|value| truncate(value, 48));
        Some(labeled_phrase(
            "Recording approval",
            "Approval recorded",
            "Could not record approval",
            action,
            phase,
        ))
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        recorded_result(&arguments)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        // Consent has been given, so the session is no longer waiting. Clearing
        // here rather than on the next turn keeps a host from showing a pause
        // that has already been answered.
        self.pending.resolve(&context.session_id);
        recorded_result(&arguments)
    }
}

fn recorded_result(arguments: &Value) -> ToolExecutionResult {
    let action = approval_action(arguments);
    ToolExecutionResult::success(json!({
        "ok": true,
        "recorded": true,
        "action": action,
        "detail": non_empty_str(arguments.get("detail")),
        "message": format!("approval recorded: {action}"),
    }))
}

/// Switches the approval level in response to how the user talks about it
/// ("be more careful", "stop asking me").
struct SetApprovalModeTool {
    modes: Arc<dyn ApprovalModeStore>,
}

#[async_trait]
impl Tool for SetApprovalModeTool {
    fn name(&self) -> &str {
        "set_approval_mode"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Set approval level")
    }

    fn description(&self) -> &str {
        "Set the soft-approval level. Use when the user asks you to be more or less cautious \
         about confirming actions. `protective` asks before any state change, `normal` asks only \
         before destructive or outward-facing actions, `off` never asks."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                // A free string rather than an `enum` so the lenient
                // `ApprovalMode::parse` synonyms stay reachable here too; a
                // hard enum would silently shadow them. Unknown values are
                // rejected in `execute`.
                "mode": {
                    "type": "string",
                    "description": "The new approval level: 'protective', 'normal', or 'off' (common synonyms like 'paranoid' or 'yolo' are also accepted)."
                }
            },
            "required": ["mode"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        approval_tool_hints()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let mode =
            arg_str(&tool_call.arguments, &["mode", "level"]).map(|value| truncate(value, 24));
        Some(labeled_phrase(
            "Setting approval level",
            "Approval level set",
            "Could not set approval level",
            mode,
            phase,
        ))
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "the approval level can only be changed from within a session",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(raw) = arguments.get("mode").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("'mode' is required");
        };
        let Some(mode) = ApprovalMode::parse(raw) else {
            return ToolExecutionResult::tool_error(format!(
                "unknown approval level '{raw}'; expected protective, normal, or off"
            ));
        };
        match self.modes.set_mode(&context.session_id, mode) {
            Ok(()) => ToolExecutionResult::success(json!({
                "ok": true,
                "mode": mode.as_str(),
                "message": format!("approval level set to {mode}; it applies from your next turn"),
            })),
            Err(error) => {
                ToolExecutionResult::tool_error(format!("could not set approval level: {error}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::SystemPromptContext;

    #[test]
    fn off_contributes_no_prompt() {
        assert!(render_approval_block(ApprovalMode::Off).is_none());
    }

    #[test]
    fn normal_and_protective_render_expected_guidance() {
        let normal = render_approval_block(ApprovalMode::Normal).expect("normal block");
        assert!(normal.starts_with("<soft_approval>"));
        assert!(normal.contains("level normal"));
        assert!(normal.contains("NORMAL"));
        // The core behaviors must be spelled out.
        assert!(normal.contains("BATCH"));
        assert!(normal.contains("record_approval"));
        // The pause is a tool call, and announcing instead of asking is the
        // failure this block exists to prevent.
        assert!(normal.contains("request_approval"));
        assert!(normal.contains("NEVER announce a critical action"));
        // A workflow the user invoked carries its own pre-authorization.
        assert!(normal.contains("a workflow the user invoked by name"));
        assert!(normal.ends_with("</soft_approval>"));

        let protective = render_approval_block(ApprovalMode::Protective).expect("protective block");
        assert!(protective.contains("level protective"));
        assert!(protective.contains("PROTECTIVE"));
    }

    #[test]
    fn capability_exposes_its_tools_undeferrable_and_read_only() {
        let capability = SoftApprovalCapability::new();
        let tools = capability.tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(
            names,
            ["request_approval", "record_approval", "set_approval_mode"]
        );
        for tool in &tools {
            assert_eq!(
                tool.deferrable_policy(),
                DeferrablePolicy::Never,
                "{} must stay directly callable",
                tool.name()
            );
            // Otherwise the hard `tool_approval` gate would ask a human to
            // approve the act of asking a human.
            assert_eq!(tool.hints().readonly, Some(true), "{}", tool.name());
            assert_eq!(tool.hints().destructive, Some(false), "{}", tool.name());
        }
    }

    #[test]
    fn config_validation_accepts_known_levels_and_rejects_typos() {
        let capability = SoftApprovalCapability::new();
        assert!(capability.validate_config(&json!({})).is_ok());
        assert!(capability.validate_config(&Value::Null).is_ok());
        assert!(
            capability
                .validate_config(&json!({"mode": "protective"}))
                .is_ok()
        );
        assert!(capability.validate_config(&json!({"mode": 3})).is_err());
        assert!(
            capability
                .validate_config(&json!({"mode": "whenever"}))
                .is_err()
        );
        assert!(capability.validate_config(&json!("normal")).is_err());
    }

    #[tokio::test]
    async fn contribution_follows_config_then_session_override() {
        let capability = SoftApprovalCapability::new();
        let session_id = SessionId::new();
        let ctx = SystemPromptContext::without_file_store(session_id);

        // Enabled with no config: the default level contributes a block.
        let default_block = capability
            .system_prompt_contribution_with_config(&ctx, &json!({}))
            .await
            .expect("default block");
        assert!(default_block.contains("level normal"));

        // Config picks the level.
        assert!(
            capability
                .system_prompt_contribution_with_config(&ctx, &json!({"mode": "off"}))
                .await
                .is_none()
        );

        // A session override wins over config, from the next turn on.
        let set = SetApprovalModeTool {
            modes: capability.modes.clone(),
        };
        let result = set
            .execute_with_context(json!({"mode": "yolo"}), &ToolContext::new(session_id))
            .await;
        assert!(result.is_success());
        assert!(
            capability
                .system_prompt_contribution_with_config(&ctx, &json!({"mode": "protective"}))
                .await
                .is_none(),
            "the session override must beat the configured level"
        );

        // …and only for that session.
        let other = SystemPromptContext::without_file_store(SessionId::new());
        assert!(
            capability
                .system_prompt_contribution_with_config(&other, &json!({"mode": "protective"}))
                .await
                .expect("other session keeps the configured level")
                .contains("level protective")
        );
    }

    #[tokio::test]
    async fn request_approval_publishes_the_pause_for_the_host() {
        let capability = SoftApprovalCapability::new();
        let pending = capability.pending();
        let session_id = SessionId::new();
        let tool = RequestApprovalTool {
            pending: pending.clone(),
        };
        assert_eq!(pending.peek(&session_id), None);

        let result = tool
            .execute_with_context(
                json!({
                    "action": "drop the staging database",
                    "question": "This deletes 12k rows and is not reversible. Go ahead?",
                }),
                &ToolContext::new(session_id),
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["awaiting_approval"], true);
        assert_eq!(
            pending.peek(&session_id),
            Some(PendingApproval {
                action: "drop the staging database".into(),
                question: "This deletes 12k rows and is not reversible. Go ahead?".into(),
            })
        );
    }

    #[tokio::test]
    async fn request_approval_without_a_question_still_reads_as_a_question() {
        let pending = PendingApprovalStore::new();
        let session_id = SessionId::new();
        RequestApprovalTool {
            pending: pending.clone(),
        }
        .execute_with_context(
            json!({"action": "deploy to production"}),
            &ToolContext::new(session_id),
        )
        .await;
        assert_eq!(
            pending.peek(&session_id).map(|p| p.question),
            Some(FALLBACK_APPROVAL_QUESTION.to_string())
        );
    }

    /// Consent ends the pause: a host must not go on telling the user it is
    /// waiting for an answer they have already given.
    #[tokio::test]
    async fn recording_an_approval_clears_the_pause_for_that_session_only() {
        let pending = PendingApprovalStore::new();
        let approved = SessionId::new();
        let untouched = SessionId::new();
        for session_id in [approved, untouched] {
            RequestApprovalTool {
                pending: pending.clone(),
            }
            .execute_with_context(
                json!({"action": "squash-merge the PR"}),
                &ToolContext::new(session_id),
            )
            .await;
        }

        let result = RecordApprovalTool {
            pending: pending.clone(),
        }
        .execute_with_context(
            json!({"action": "squash-merge the PR", "detail": "CI green"}),
            &ToolContext::new(approved),
        )
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["action"], "squash-merge the PR");
        assert_eq!(value["detail"], "CI green");
        assert_eq!(pending.peek(&approved), None);
        assert!(pending.peek(&untouched).is_some());
    }

    /// Models do emit a bare `record_approval()` after spoken consent. Failing
    /// there would turn an approval that was actually given into an error.
    #[tokio::test]
    async fn record_approval_accepts_empty_arguments() {
        let result = RecordApprovalTool {
            pending: PendingApprovalStore::new(),
        }
        .execute_with_context(json!({}), &ToolContext::new(SessionId::new()))
        .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["action"], FALLBACK_APPROVAL_ACTION);
        assert!(value["detail"].is_null());
    }

    #[tokio::test]
    async fn set_approval_mode_rejects_an_unknown_level() {
        let capability = SoftApprovalCapability::new();
        let tool = SetApprovalModeTool {
            modes: capability.modes.clone(),
        };
        let session_id = SessionId::new();
        assert!(
            tool.execute_with_context(json!({"mode": "whenever"}), &ToolContext::new(session_id))
                .await
                .is_error()
        );
        assert!(
            tool.execute_with_context(json!({}), &ToolContext::new(session_id))
                .await
                .is_error()
        );
        assert_eq!(capability.modes.mode(&session_id), None);
    }

    /// The seam a host with its own durable setting migrates onto.
    #[tokio::test]
    async fn a_host_mode_store_overrides_config_and_receives_writes() {
        #[derive(Default)]
        struct HostSetting(Mutex<Option<ApprovalMode>>);

        impl ApprovalModeStore for HostSetting {
            fn mode(&self, _session_id: &SessionId) -> Option<ApprovalMode> {
                *self.0.lock().unwrap()
            }
            fn set_mode(&self, _session_id: &SessionId, mode: ApprovalMode) -> Result<(), String> {
                *self.0.lock().unwrap() = Some(mode);
                Ok(())
            }
        }

        let setting = Arc::new(HostSetting::default());
        let capability = SoftApprovalCapability::with_mode_store(setting.clone());
        let session_id = SessionId::new();
        let ctx = SystemPromptContext::without_file_store(session_id);

        // Empty host setting falls through to config.
        assert!(
            capability
                .system_prompt_contribution_with_config(&ctx, &json!({"mode": "protective"}))
                .await
                .expect("configured block")
                .contains("level protective")
        );

        SetApprovalModeTool {
            modes: capability.modes.clone(),
        }
        .execute_with_context(json!({"mode": "off"}), &ToolContext::new(session_id))
        .await;

        assert_eq!(*setting.0.lock().unwrap(), Some(ApprovalMode::Off));
        assert!(
            capability
                .system_prompt_contribution_with_config(&ctx, &json!({"mode": "protective"}))
                .await
                .is_none(),
            "the host setting wins over config, in every session it covers"
        );
    }
}
