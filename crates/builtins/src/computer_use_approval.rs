// Hard per-call approval for computer use in hosted sessions (EVE-1133,
// TM-TOOL-008).
//
// A `computer` call that commits input (types text, presses Enter, leaves the
// page; see `everruns_core::computer_use::action_requires_approval`), or that
// carries provider safety checks (OpenAI's `pending_safety_checks`), does not
// run until a person approves that exact call. Pointer moves, clicks, scrolls
// and screenshots run freely: gating them would make computer use unusable.
//
// Decision: this is the `tool_approval` gate from EVE-1140 with a policy, not
// a second mechanism. The durable approver parks the turn, the tool-approvals
// API records the answer, and the retried call is matched by fingerprint, so
// the card, the deadline and the fail-closed behavior are the ones every
// other hosted approval has. It is contributed by the computer use capability
// itself, so it applies whether or not the agent also enables
// `tool_approval`; when it does, one answer carries the call through both
// gates (see `tool_approval_durable`).
//
// Decision: an "always allow" answer for `computer` is honored. A person who
// chose it has knowingly turned the per-call prompt off for the session; a
// gate that silently re-asks after that answer would only teach people to
// click through.
//
// Spec: knowledge/execution/computer-use.md.

use std::sync::Arc;

use everruns_core::computer_use::{COMPUTER_TOOL_NAME, computer_call_requires_approval};

use crate::tool_approval::{ApprovalMode, DurableToolApprover, ToolApprovalCapability};
use crate::tool_hooks::PreToolUseHook;
use crate::tool_types::{ToolCall, ToolDefinition};

/// Whether `tool_call` is a `computer` call the hard gate must hold.
pub fn computer_call_is_gated(tool_call: &ToolCall) -> bool {
    tool_call.name == COMPUTER_TOOL_NAME
        && computer_call_requires_approval(&tool_call.execution_arguments())
}

/// The hosted hard gate for computer use: a durable `tool_approval` gate that
/// asks exactly for [`computer_call_is_gated`] calls.
pub fn computer_use_approval_hook() -> Arc<dyn PreToolUseHook> {
    ToolApprovalCapability::new(Arc::new(DurableToolApprover))
        .with_policy(Arc::new(|tool_call: &ToolCall, _: &ToolDefinition| {
            computer_call_is_gated(tool_call)
        }))
        // The mode is ignored under a policy; it only labels the request.
        .hook(ApprovalMode::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_approval::{
        StoredToolApproval, always_decision_storage_key, one_off_decision_storage_key,
    };
    use crate::tool_hooks::PreToolUseDecision;
    use crate::tool_types::{BuiltinTool, ToolApprovalRequired};
    use crate::typed_id::SessionId;
    use chrono::Utc;
    use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
    use everruns_core::tool_context::ToolContext;
    use everruns_provider::error::Result as StoreResult;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<String, String>>);

    impl MemoryStore {
        fn put(&self, key: String, record: &StoredToolApproval) {
            self.0
                .lock()
                .unwrap()
                .insert(key, serde_json::to_string(record).unwrap());
        }
    }

    #[async_trait::async_trait]
    impl SessionStorageStore for MemoryStore {
        async fn set_value(&self, _: SessionId, key: &str, value: &str) -> StoreResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        async fn get_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        async fn take_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
            Ok(self.0.lock().unwrap().remove(key))
        }
        async fn delete_value(&self, _: SessionId, key: &str) -> StoreResult<bool> {
            Ok(self.0.lock().unwrap().remove(key).is_some())
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

    fn computer_tool() -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: COMPUTER_TOOL_NAME.to_string(),
            display_name: Some("Computer".to_string()),
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })
    }

    fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }
    }

    async fn decide(
        call: ToolCall,
        session: SessionId,
        store: &Arc<MemoryStore>,
    ) -> PreToolUseDecision {
        let context = ToolContext::new(session).with_storage_store_arc(store.clone());
        computer_use_approval_hook()
            .before_exec(call, &computer_tool(), &context)
            .await
    }

    fn deferred(decision: PreToolUseDecision) -> ToolApprovalRequired {
        match decision {
            PreToolUseDecision::Defer { result, .. } => {
                ToolApprovalRequired::from_tool_result(&result).expect("structured payload")
            }
            other => panic!("expected a deferral, got {other:?}"),
        }
    }

    #[test]
    fn only_committing_computer_calls_are_gated() {
        for (name, arguments) in [
            ("computer", json!({"action": "type", "text": "card number"})),
            ("computer", json!({"action": "key", "text": "Return"})),
            (
                "computer",
                json!({"action": "navigate", "url": "https://example.com"}),
            ),
            (
                "computer",
                json!({"actions": [{"action": "left_click", "coordinate": [1, 1]},
                                   {"action": "type", "text": "x"}]}),
            ),
            (
                "computer",
                json!({"actions": [{"action": "screenshot"}],
                       "pending_safety_checks": [{"id": "sc_1"}]}),
            ),
        ] {
            assert!(
                computer_call_is_gated(&call("c", name, arguments.clone())),
                "{arguments}"
            );
        }
        for (name, arguments) in [
            (
                "computer",
                json!({"action": "left_click", "coordinate": [1, 1]}),
            ),
            ("computer", json!({"action": "screenshot"})),
            ("computer", json!({"action": "key", "text": "Tab"})),
            // Another tool with the same argument shape is not this gate's.
            ("web_form", json!({"action": "type", "text": "x"})),
        ] {
            assert!(
                !computer_call_is_gated(&call("c", name, arguments.clone())),
                "{arguments}"
            );
        }
    }

    #[tokio::test]
    async fn a_flagged_action_parks_until_approved_once() {
        let store = Arc::new(MemoryStore::default());
        let session = SessionId::new_random();
        let typing = json!({"action": "type", "text": "Ada"});

        let request =
            deferred(decide(call("c1", "computer", typing.clone()), session, &store).await);
        assert_eq!(request.tool, "computer");
        assert_eq!(request.risk, "policy");
        assert_eq!(request.arguments, typing);

        store.put(
            one_off_decision_storage_key(&request.fingerprint),
            &StoredToolApproval::one_off("computer", &request.fingerprint, true, Utc::now()),
        );
        let decision = decide(call("c2", "computer", typing.clone()), session, &store).await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));

        // Used once: typing the same text again asks again.
        deferred(decide(call("c3", "computer", typing), session, &store).await);
    }

    #[tokio::test]
    async fn approving_one_action_does_not_approve_another() {
        let store = Arc::new(MemoryStore::default());
        let session = SessionId::new_random();
        let request = deferred(
            decide(
                call("c1", "computer", json!({"action": "type", "text": "Ada"})),
                session,
                &store,
            )
            .await,
        );
        store.put(
            one_off_decision_storage_key(&request.fingerprint),
            &StoredToolApproval::one_off("computer", &request.fingerprint, true, Utc::now()),
        );
        deferred(
            decide(
                call("c2", "computer", json!({"action": "type", "text": "Bob"})),
                session,
                &store,
            )
            .await,
        );
    }

    #[tokio::test]
    async fn a_rejection_blocks_and_free_actions_never_ask() {
        let store = Arc::new(MemoryStore::default());
        let session = SessionId::new_random();
        let enter = json!({"action": "key", "text": "Return"});
        let request =
            deferred(decide(call("c1", "computer", enter.clone()), session, &store).await);
        store.put(
            one_off_decision_storage_key(&request.fingerprint),
            &StoredToolApproval::one_off("computer", &request.fingerprint, false, Utc::now()),
        );
        let decision = decide(call("c2", "computer", enter), session, &store).await;
        assert!(matches!(decision, PreToolUseDecision::Block { .. }));

        let click = json!({"action": "left_click", "coordinate": [10, 10]});
        let decision = decide(call("c3", "computer", click), session, &store).await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        assert!(
            store.0.lock().unwrap().is_empty(),
            "nothing recorded for free actions"
        );
    }

    #[tokio::test]
    async fn an_always_answer_is_honored_and_no_storage_fails_closed() {
        let store = Arc::new(MemoryStore::default());
        let session = SessionId::new_random();
        store.put(
            always_decision_storage_key("computer"),
            &StoredToolApproval::always("computer", true, Utc::now()),
        );
        let typing = json!({"action": "type", "text": "Ada"});
        let decision = decide(call("c1", "computer", typing.clone()), session, &store).await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));

        // No session storage: nobody can be asked, so the call is blocked.
        let decision = computer_use_approval_hook()
            .before_exec(
                call("c2", "computer", typing),
                &computer_tool(),
                &ToolContext::new(session),
            )
            .await;
        assert!(matches!(decision, PreToolUseDecision::Block { .. }));
    }
}
