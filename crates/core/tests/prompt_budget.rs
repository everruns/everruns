// Prompt-size budgets for capabilities used by the example coding CLI
// and similar small surfaces. The goal is to keep first-turn prompt
// overhead bounded so simple tasks do not pay for unnecessary boilerplate.
//
// Each test measures the *actual* contribution that ends up in the
// assembled prompt — `system_prompt_contribution(&ctx)` for default
// capabilities, which includes the `<capability id="…">…</capability>`
// wrapping that `apply_capabilities` injects.
//
// Ratcheting: if you intentionally need more bytes, bump the cap in the
// same PR and explain why in the commit message. Lowering a cap is
// always fine.

use everruns_core::capabilities::{
    BudgetingCapability, Capability, DataKnowledgeCapability, FileSystemCapability,
    InfinityContextCapability, MemoryCapability, MessageMetadataCapability, SampleDataCapability,
    SelfBudgetCapability, SessionSandboxCapability, SkillsCapability, StatelessTodoListCapability,
    SubagentCapability, SystemPromptContext, WebFetchCapability,
};
use everruns_core::typed_id::SessionId;

async fn assert_contribution_under(cap: &dyn Capability, max_bytes: usize) {
    let ctx = SystemPromptContext::without_file_store(SessionId::new());
    let prompt = cap
        .system_prompt_contribution(&ctx)
        .await
        .unwrap_or_else(|| panic!("{} did not contribute a prompt", cap.id()));
    assert!(
        prompt.len() <= max_bytes,
        "{}: contribution is {} bytes (~{} tokens), cap is {} bytes",
        cap.id(),
        prompt.len(),
        prompt.len() / 4,
        max_bytes,
    );
}

#[tokio::test]
async fn stateless_todo_list_prompt_within_budget() {
    assert_contribution_under(&StatelessTodoListCapability, 450).await;
}

#[tokio::test]
async fn file_system_prompt_within_budget() {
    assert_contribution_under(&FileSystemCapability, 950).await;
}

#[tokio::test]
async fn infinity_context_prompt_within_budget() {
    assert_contribution_under(&InfinityContextCapability, 450).await;
}

#[tokio::test]
async fn skills_static_prompt_within_budget() {
    assert_contribution_under(&SkillsCapability, 250).await;
}

#[tokio::test]
async fn memory_prompt_is_empty() {
    // The mounts-based Memory capability surfaces data through the
    // filesystem, not the system prompt; keep its prompt cost at zero.
    let ctx = SystemPromptContext::without_file_store(SessionId::new());
    assert!(
        MemoryCapability
            .system_prompt_contribution(&ctx)
            .await
            .is_none(),
        "memory is expected to contribute no prompt"
    );
}

#[tokio::test]
async fn subagents_prompt_within_budget() {
    assert_contribution_under(&SubagentCapability, 350).await;
}

#[tokio::test]
async fn message_metadata_prompt_within_budget() {
    assert_contribution_under(&MessageMetadataCapability, 350).await;
}

#[tokio::test]
async fn budgeting_prompt_within_budget() {
    assert_contribution_under(&BudgetingCapability, 300).await;
}

#[tokio::test]
async fn self_budget_prompt_within_budget() {
    assert_contribution_under(&SelfBudgetCapability, 475).await;
}

#[tokio::test]
async fn session_sandbox_prompt_within_budget() {
    assert_contribution_under(&SessionSandboxCapability, 300).await;
}

#[tokio::test]
async fn mounted_data_prompts_within_budget() {
    assert_contribution_under(&SampleDataCapability, 250).await;
    // Bumped 300 → 350: the Open Knowledge Format (OKF) adoption (#2321) added a
    // short description of the mounted bundle layout (markdown + YAML frontmatter,
    // index.md navigation) to the system-prompt contribution, taking it to 342
    // bytes. The content is intentional first-turn guidance, so ratchet the cap up
    // per this file's policy rather than trimming it.
    assert_contribution_under(&DataKnowledgeCapability, 350).await;
}

#[tokio::test]
async fn web_fetch_prompt_within_budget() {
    // `web_fetch` uses the dynamic contribution path because its prompt
    // depends on the `enable_file_download` flag. Check both branches —
    // both go through `<capability id="…">…</capability>` wrapping.
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
