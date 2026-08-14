// Prompt-size budget for the fixture capabilities, mirroring
// crates/core/tests/prompt_budget.rs for capabilities that moved to
// test-support (EVE-875). Ratcheting policy is the same: bump a cap only
// deliberately, in the same PR, with the reason in the commit message.

use everruns_core::capabilities::{Capability, SystemPromptContext};
use everruns_provider::typed_id::SessionId;
use everruns_test_support::SampleDataCapability;

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
async fn sample_data_prompt_within_budget() {
    assert_contribution_under(&SampleDataCapability, 250).await;
}
