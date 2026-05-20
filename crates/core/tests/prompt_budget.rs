// Static prompt-size budgets for capabilities used by the example coding
// CLI and similar small surfaces. The goal is to keep first-turn prompt
// overhead bounded so simple tasks do not pay for unnecessary boilerplate.
//
// Ratcheting: if you intentionally need more bytes, bump the cap in the
// same PR and explain why in the commit message. Lowering a cap is always
// fine.

use everruns_core::capabilities::{
    Capability, FileSystemCapability, InfinityContextCapability, SkillsCapability,
    StatelessTodoListCapability, SystemPromptContext, WebFetchCapability,
};
use everruns_core::typed_id::SessionId;

fn assert_prompt_under(cap: &dyn Capability, max_bytes: usize) {
    let prompt = cap
        .system_prompt_addition()
        .unwrap_or_else(|| panic!("{} has no system_prompt_addition", cap.id()));
    assert!(
        prompt.len() <= max_bytes,
        "{}: system_prompt_addition is {} bytes (~{} tokens), cap is {} bytes",
        cap.id(),
        prompt.len(),
        prompt.len() / 4,
        max_bytes,
    );
}

#[test]
fn stateless_todo_list_prompt_within_budget() {
    assert_prompt_under(&StatelessTodoListCapability, 400);
}

#[test]
fn file_system_prompt_within_budget() {
    assert_prompt_under(&FileSystemCapability, 900);
}

#[test]
fn infinity_context_prompt_within_budget() {
    assert_prompt_under(&InfinityContextCapability, 400);
}

#[test]
fn skills_static_prompt_within_budget() {
    assert_prompt_under(&SkillsCapability, 200);
}

#[tokio::test]
async fn web_fetch_prompt_within_budget() {
    // `web_fetch` uses the dynamic contribution path because its prompt
    // depends on the `enable_file_download` flag. Check both branches.
    let cap = WebFetchCapability::new(None);
    let ctx = SystemPromptContext::without_file_store(SessionId::new());

    let disabled = cap
        .system_prompt_contribution_with_config(&ctx, &serde_json::json!({}))
        .await
        .expect("web_fetch contributes a prompt");
    assert!(
        disabled.len() <= 250,
        "web_fetch (no file download): {} bytes",
        disabled.len()
    );

    let enabled = cap
        .system_prompt_contribution_with_config(
            &ctx,
            &serde_json::json!({"enable_file_download": true}),
        )
        .await
        .expect("web_fetch contributes a prompt");
    assert!(
        enabled.len() <= 350,
        "web_fetch (file download enabled): {} bytes",
        enabled.len()
    );
}
