// Durable decisions for the `tool_approval` gate (EVE-1140).
//
// Split out of `tool_approval.rs`, which owns the gate itself. This half owns
// what a hosted runtime needs instead of a blocking approver: where a person's
// answer is recorded, how a retried call finds it, and the fingerprint that
// binds a one-off answer to the exact call that was shown.
// Spec: knowledge/execution/tool-approval.md.

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::builtins::tool_approval::{ApprovalDecision, ToolApprover};
use crate::builtins::tool_types::{ToolCall, ToolDefinition};
use crate::builtins::typed_id::SessionId;
use crate::tool_context::ToolContext;

/// Session-storage prefix every durable approval record lives under.
///
/// Reserved from the model-facing `kv_store` tool and the session storage
/// listing (`is_internal_session_kv_key`), so neither a model nor a tool can
/// mint an approval for itself. Only the server's tool-approvals API writes it.
pub use crate::capabilities::TOOL_APPROVAL_KV_PREFIX;

/// How long a one-off answer waits for the retried call before it lapses.
///
/// A one-off approval is meant for the call the person just looked at, retried
/// in the turn that resumes; one left lying around should not quietly let an
/// identical call through much later.
pub const ONE_OFF_DECISION_TTL_SECONDS: i64 = 3_600;

/// How long a consumed one-off approval keeps answering for the same call.
const CONSUMED_ONE_OFF_TTL: Duration = Duration::from_secs(60);

/// Most consumed one-off approvals remembered per process.
const CONSUMED_ONE_OFF_CAPACITY: usize = 512;

/// One-off approvals this process consumed, by (session, call id, fingerprint).
///
/// Decision: one call can pass two durable gates (the agent's `tool_approval`
/// and a capability's own hard gate). Each takes the
/// one-off answer with a destructive read, so without this the second gate
/// would find nothing, defer, and every answer would be spent by whichever
/// gate ran first: the call could never run. The gates for one call run back
/// to back in one process, so a short, bounded in-process memo is enough; a
/// different call (a new id) or a later retry still needs its own answer.
static CONSUMED_ONE_OFF: LazyLock<Mutex<VecDeque<ConsumedOneOff>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

/// (session, call id, fingerprint, when it was consumed).
type ConsumedOneOff = (SessionId, String, String, Instant);

fn consumed_one_off(session_id: SessionId, call_id: &str, fingerprint: &str) -> bool {
    let mut consumed = CONSUMED_ONE_OFF
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    consumed.retain(|(.., at)| at.elapsed() < CONSUMED_ONE_OFF_TTL);
    consumed
        .iter()
        .any(|(session, id, fp, _)| *session == session_id && id == call_id && fp == fingerprint)
}

fn record_consumed_one_off(session_id: SessionId, call_id: &str, fingerprint: &str) {
    let mut consumed = CONSUMED_ONE_OFF
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if consumed.len() >= CONSUMED_ONE_OFF_CAPACITY {
        consumed.pop_front();
    }
    consumed.push_back((
        session_id,
        call_id.to_string(),
        fingerprint.to_string(),
        Instant::now(),
    ));
}

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
        store: &dyn crate::session_services::SessionStorageStore,
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
        if consumed_one_off(session_id, &tool_call.id, &fingerprint) {
            return Ok(ApprovalDecision::Allow);
        }
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
            if !record.allow {
                return Ok(ApprovalDecision::Reject);
            }
            record_consumed_one_off(session_id, &tool_call.id, &fingerprint);
            return Ok(ApprovalDecision::Allow);
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
    use crate::builtins::capabilities::Capability;
    use crate::builtins::tool_approval::{
        ApprovalMode, DEFAULT_APPROVAL_TIMEOUT_SECONDS, ToolApprovalCapability,
        approval_timeout_from_config,
    };
    use crate::builtins::tool_hooks::{PreToolUseDecision, PreToolUseHook};
    use crate::builtins::tool_types::{BuiltinTool, ToolApprovalRequired, ToolHints};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

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

    // ------------------------------------------------------------------
    // Durable approvals (EVE-1140)
    // ------------------------------------------------------------------

    mod durable {
        use super::*;
        use crate::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
        use everruns_contracts::error::Result as StoreResult;

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
                    Err(everruns_contracts::error::AgentLoopError::Internal(
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

            // Consumed: the model's next identical call (a new call id, the
            // same arguments) needs a fresh approval.
            let again = ToolCall {
                id: "call_a_again".to_string(),
                ..send("a")
            };
            let decision = resumed
                .before_exec(again, &open_world_tool(), &context(session, &store))
                .await;
            deferred_request(decision);
        }

        #[tokio::test]
        async fn one_answer_lets_a_call_through_two_durable_gates() {
            // Two `tool_approval` gates (agent-level and capability-level) can
            // both gate one call. One one-off answer must
            // carry it through both, in either order, or it could never run.
            let store = Arc::new(MemoryStore::default());
            let session = SessionId::new_random();
            let general = fresh_hook(ApprovalMode::Normal);
            let hard = ToolApprovalCapability::new(Arc::new(DurableToolApprover))
                .with_policy(Arc::new(|_: &ToolCall, _: &ToolDefinition| true))
                .hook(ApprovalMode::Off);
            let call = send("both");
            let request = deferred_request(
                general
                    .before_exec(call.clone(), &open_world_tool(), &context(session, &store))
                    .await,
            );
            store.put(
                one_off_decision_storage_key(&request.fingerprint),
                &StoredToolApproval::one_off("t", &request.fingerprint, true, Utc::now()),
            );

            for gate in [&general, &hard] {
                let decision = gate
                    .before_exec(call.clone(), &open_world_tool(), &context(session, &store))
                    .await;
                assert!(matches!(decision, PreToolUseDecision::Continue(_)));
            }

            // It does not leak to another call, or another session.
            let other_call = ToolCall {
                id: "call_both_retry".to_string(),
                ..send("both")
            };
            deferred_request(
                hard.before_exec(other_call, &open_world_tool(), &context(session, &store))
                    .await,
            );
            deferred_request(
                hard.before_exec(
                    call,
                    &open_world_tool(),
                    &context(SessionId::new_random(), &store),
                )
                .await,
            );
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
