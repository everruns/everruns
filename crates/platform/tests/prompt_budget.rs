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

use everruns_builtins::{InfinityContextCapability, SkillsCapability};
use everruns_core::capabilities::{Capability, SystemPromptContext};
use everruns_platform::capabilities::SessionSandboxCapability;
use everruns_platform::capabilities::{
    DataKnowledgeCapability, MemoryCapability, SubagentCapability,
};
use everruns_provider::typed_id::SessionId;

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
    // Bumped 350 → 500: background-by-default spawning needs first-turn
    // guidance on the task_id/wake workflow and the foreground opt-out, or
    // models block on results they no longer receive inline.
    assert_contribution_under(&SubagentCapability, 500).await;
}

#[tokio::test]
async fn session_sandbox_prompt_within_budget() {
    assert_contribution_under(&SessionSandboxCapability, 300).await;
}

// Note: the `sample_data` fixture capability's budget test lives in
// `everruns-test-support` (tests/prompt_budget_fixtures.rs) since the fixture
// moved out of core (EVE-875).
#[tokio::test]
async fn mounted_data_prompts_within_budget() {
    // Bumped 300 → 350: the Open Knowledge Format (OKF) adoption (#2321) added a
    // short description of the mounted bundle layout (markdown + YAML frontmatter,
    // index.md navigation) to the system-prompt contribution, taking it to 342
    // bytes. The content is intentional first-turn guidance, so ratchet the cap up
    // per this file's policy rather than trimming it.
    assert_contribution_under(&DataKnowledgeCapability, 350).await;
}
