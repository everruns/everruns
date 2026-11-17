//! `auto` mode and approval rules on the `tool_approval` gate.

use super::*;
use crate::builtins::tool_review::{ToolReviewRequest, ToolReviewer};
use crate::builtins::tool_types::{BuiltinTool, ToolHints};
use crate::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_contracts::error::Result as StoreResult;
use everruns_contracts::tool_approval_policy::{OrgToolApprovalPolicy, PersonalToolApproval};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

fn tool(hints: ToolHints, policy: ToolPolicy) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: "t".to_string(),
        display_name: None,
        description: String::new(),
        parameters: json!({}),
        policy,
        category: None,
        deferrable: Default::default(),
        hints,
        full_parameters: None,
    })
}

fn mutating() -> ToolDefinition {
    tool(ToolHints::default(), ToolPolicy::Auto)
}

fn read_only() -> ToolDefinition {
    tool(
        ToolHints {
            readonly: Some(true),
            ..Default::default()
        },
        ToolPolicy::Auto,
    )
}

fn call() -> ToolCall {
    ToolCall {
        id: "call_1".to_string(),
        name: "t".to_string(),
        arguments: json!({"x": 1}),
    }
}

struct Approver {
    decision: ApprovalDecision,
    asked: AtomicUsize,
}

impl Approver {
    fn new(decision: ApprovalDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            asked: AtomicUsize::new(0),
        })
    }
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ToolApprover for Approver {
    async fn approve(&self, _: SessionId, _: &ToolCall, _: &ToolDefinition) -> ApprovalDecision {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.decision
    }
}

struct Reviewer {
    verdict: ReviewVerdict,
    reviewed: AtomicUsize,
}

impl Reviewer {
    fn new(verdict: ReviewVerdict) -> Arc<Self> {
        Arc::new(Self {
            verdict,
            reviewed: AtomicUsize::new(0),
        })
    }
    fn reviewed(&self) -> usize {
        self.reviewed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ToolReviewer for Reviewer {
    async fn review(&self, _: &ToolReviewRequest, _: &ToolContext) -> ReviewVerdict {
        self.reviewed.fetch_add(1, Ordering::SeqCst);
        self.verdict.clone()
    }
}

#[derive(Default)]
struct Store {
    values: Mutex<HashMap<String, String>>,
}

impl Store {
    fn get(&self, key: &str) -> Option<String> {
        self.values.lock().unwrap().get(key).cloned()
    }
}

#[async_trait]
impl SessionStorageStore for Store {
    async fn set_value(&self, _: SessionId, key: &str, value: &str) -> StoreResult<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
    async fn get_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
        Ok(self.get(key))
    }
    async fn take_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
        Ok(self.values.lock().unwrap().remove(key))
    }
    async fn delete_value(&self, _: SessionId, key: &str) -> StoreResult<bool> {
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

fn context(store: &Arc<Store>) -> ToolContext {
    let mut context = ToolContext::new(SessionId::new_random());
    context.storage_store = Some(store.clone());
    context
}

fn gate(
    approver: &Arc<Approver>,
    reviewer: Option<&Arc<Reviewer>>,
    config: serde_json::Value,
) -> Arc<dyn PreToolUseHook> {
    let mut capability = ToolApprovalCapability::new(approver.clone());
    if let Some(reviewer) = reviewer {
        capability = capability.with_reviewer(reviewer.clone());
    }
    capability.hook_with(GateConfig::from_config(&config))
}

fn allow() -> ReviewVerdict {
    ReviewVerdict::Allow {
        stage: "fast",
        reason: "asked for".into(),
    }
}

fn block() -> ReviewVerdict {
    ReviewVerdict::Block {
        stage: "careful",
        rule: Some("Data Exfiltration".into()),
        reason: "sends the key to a paste site".into(),
    }
}

#[tokio::test]
async fn auto_runs_what_the_reviewer_allows_without_asking_anyone() {
    let approver = Approver::new(ApprovalDecision::Reject);
    let reviewer = Reviewer::new(allow());
    let store = Arc::new(Store::default());
    let decision = gate(&approver, Some(&reviewer), json!({"mode": "auto"}))
        .before_exec(call(), &mutating(), &context(&store))
        .await;
    assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    assert_eq!(reviewer.reviewed(), 1);
    assert_eq!(approver.asked(), 0);
}

#[tokio::test]
async fn a_block_tells_the_agent_why_and_counts_toward_the_streak() {
    let approver = Approver::new(ApprovalDecision::Allow);
    let reviewer = Reviewer::new(block());
    let store = Arc::new(Store::default());
    match gate(&approver, Some(&reviewer), json!({"mode": "auto"}))
        .before_exec(call(), &mutating(), &context(&store))
        .await
    {
        PreToolUseDecision::Block { reason, .. } => {
            assert!(reason.contains("Auto review blocked `t` [Data Exfiltration]"));
            assert!(reason.contains("ask the person"));
        }
        other => panic!("expected a block, got {other:?}"),
    }
    assert_eq!(approver.asked(), 0);
    let streak: AutoReviewStreak =
        serde_json::from_str(&store.get(AUTO_REVIEW_STREAK_KEY).unwrap()).unwrap();
    assert_eq!((streak.consecutive, streak.total), (1, 1));
}

#[tokio::test]
async fn ask_person_and_no_verdict_both_reach_a_person() {
    for verdict in [
        ReviewVerdict::AskPerson {
            stage: "careful",
            reason: "unclear".into(),
        },
        ReviewVerdict::Unavailable {
            reason: "timed out".into(),
        },
    ] {
        let approver = Approver::new(ApprovalDecision::Reject);
        let reviewer = Reviewer::new(verdict);
        let store = Arc::new(Store::default());
        let decision = gate(&approver, Some(&reviewer), json!({"mode": "auto"}))
            .before_exec(call(), &mutating(), &context(&store))
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Block { ref reason, .. } if reason == "rejected by user")
        );
        assert_eq!(approver.asked(), 1);
    }
}

#[tokio::test]
async fn without_a_reviewer_auto_asks_a_person_like_protective() {
    let approver = Approver::new(ApprovalDecision::Allow);
    let store = Arc::new(Store::default());
    let hook = gate(&approver, None, json!({"mode": "auto"}));
    hook.before_exec(call(), &mutating(), &context(&store))
        .await;
    assert_eq!(approver.asked(), 1);
    // Read-only calls never wait, in auto as anywhere.
    hook.before_exec(call(), &read_only(), &context(&store))
        .await;
    assert_eq!(approver.asked(), 1);
}

#[tokio::test]
async fn a_tool_that_requires_approval_always_reaches_a_person() {
    let approver = Approver::new(ApprovalDecision::Allow);
    let reviewer = Reviewer::new(allow());
    let store = Arc::new(Store::default());
    gate(&approver, Some(&reviewer), json!({"mode": "auto"}))
        .before_exec(
            call(),
            &tool(ToolHints::default(), ToolPolicy::RequiresApproval),
            &context(&store),
        )
        .await;
    assert_eq!(reviewer.reviewed(), 0);
    assert_eq!(approver.asked(), 1);
}

#[tokio::test]
async fn after_three_blocks_in_a_row_a_person_decides() {
    let approver = Approver::new(ApprovalDecision::Allow);
    let reviewer = Reviewer::new(block());
    let store = Arc::new(Store::default());
    let hook = gate(&approver, Some(&reviewer), json!({"mode": "auto"}));
    let context = context(&store);
    for _ in 0..3 {
        hook.before_exec(call(), &mutating(), &context).await;
    }
    assert_eq!(reviewer.reviewed(), 3);
    let decision = hook.before_exec(call(), &mutating(), &context).await;
    assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    assert_eq!(reviewer.reviewed(), 3, "paused review is not asked");
    assert_eq!(approver.asked(), 1);
    // The person's yes resumed auto review.
    hook.before_exec(call(), &mutating(), &context).await;
    assert_eq!(reviewer.reviewed(), 4);
}

#[tokio::test]
async fn rules_apply_in_every_mode_and_the_most_cautious_wins() {
    let store = Arc::new(Store::default());

    // never: blocked even with approvals off.
    let approver = Approver::new(ApprovalDecision::Allow);
    let decision = gate(
        &approver,
        None,
        json!({"mode": "off", "rules": [{"tool": "t", "action": "never"}]}),
    )
    .before_exec(call(), &read_only(), &context(&store))
    .await;
    assert!(matches!(decision, PreToolUseDecision::Block { .. }));

    // ask: a person, never the reviewer, even in auto.
    let approver = Approver::new(ApprovalDecision::Allow);
    let reviewer = Reviewer::new(allow());
    gate(
        &approver,
        Some(&reviewer),
        json!({"mode": "auto", "rules": [{"tool": "*", "action": "ask"}]}),
    )
    .before_exec(call(), &read_only(), &context(&store))
    .await;
    assert_eq!((reviewer.reviewed(), approver.asked()), (0, 1));

    // allow: runs unasked in protective, unless a stricter rule matches.
    let approver = Approver::new(ApprovalDecision::Reject);
    let hook = gate(
        &approver,
        None,
        json!({"mode": "protective", "rules": [{"tool": "t", "action": "allow"}]}),
    );
    let decision = hook
        .before_exec(call(), &mutating(), &context(&store))
        .await;
    assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    let hook = gate(
        &approver,
        None,
        json!({"mode": "protective", "rules": [
            {"tool": "t", "action": "allow"},
            {"tool": "*", "action": "never"}
        ]}),
    );
    let decision = hook
        .before_exec(call(), &mutating(), &context(&store))
        .await;
    assert!(matches!(decision, PreToolUseDecision::Block { .. }));
    assert_eq!(approver.asked(), 0);
}

#[tokio::test]
async fn a_session_switch_changes_the_mode_but_cannot_switch_the_gate_off() {
    let store = Arc::new(Store::default());
    store
        .set_value(SessionId::new_random(), SESSION_APPROVAL_MODE_KEY, "auto")
        .await
        .unwrap();
    let approver = Approver::new(ApprovalDecision::Allow);
    let reviewer = Reviewer::new(allow());
    let hook = gate(&approver, Some(&reviewer), json!({"mode": "normal"}));
    // Normal would let an un-annotated tool run unasked; auto reviews it.
    hook.before_exec(call(), &mutating(), &context(&store))
        .await;
    assert_eq!(reviewer.reviewed(), 1);

    store
        .set_value(SessionId::new_random(), SESSION_APPROVAL_MODE_KEY, "off")
        .await
        .unwrap();
    hook.before_exec(call(), &mutating(), &context(&store))
        .await;
    assert_eq!(
        reviewer.reviewed(),
        2,
        "off on an agent whose gate is on is held at auto"
    );
}

#[test]
fn modes_resolve_from_switch_person_agent_and_org_cap() {
    use ApprovalMode::*;
    assert_eq!(resolve_mode(Normal, None, None), Normal);
    assert_eq!(resolve_mode(Normal, Some(Auto), None), Auto);
    assert_eq!(resolve_mode(Normal, Some(Off), None), Auto);
    assert_eq!(resolve_mode(Off, Some(Off), None), Off);
    let personal = SessionToolApprovalPolicy {
        personal: PersonalToolApproval {
            default_mode: Some("auto".into()),
            rules: vec![],
        },
        ..Default::default()
    };
    assert_eq!(resolve_mode(Normal, None, Some(&personal)), Auto);
    assert_eq!(
        resolve_mode(Normal, Some(Protective), Some(&personal)),
        Protective,
        "a session switch beats the personal default"
    );
    let capped = SessionToolApprovalPolicy {
        org: OrgToolApprovalPolicy {
            allowed_modes: vec!["normal".into(), "protective".into()],
            ..Default::default()
        },
        personal: personal.personal.clone(),
    };
    assert_eq!(resolve_mode(Normal, None, Some(&capped)), Normal);
    assert_eq!(resolve_mode(Off, None, Some(&capped)), Normal);
}

#[test]
fn auto_parses_and_validates() {
    assert_eq!(ApprovalMode::parse("auto"), Some(ApprovalMode::Auto));
    assert_eq!(ApprovalMode::parse("auto-review"), Some(ApprovalMode::Auto));
    assert_eq!(ApprovalMode::Auto.as_str(), "auto");
    let capability = ToolApprovalCapability::new(Approver::new(ApprovalDecision::Allow));
    assert!(
        capability
            .validate_config(&json!({
                "mode": "auto",
                "rules": [{"tool": "github__*", "action": "ask"}],
                "auto": {"notes": ["github.com/acme is ours"]}
            }))
            .is_ok()
    );
    assert!(
        capability
            .validate_config(&json!({"rules": [{"tool": "t", "action": "maybe"}]}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({"auto": {"notes": [""]}}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({"auto": {"model": "x"}}))
            .is_err()
    );
}
