//! Prompt-size ratchets for portable policy capabilities.

use everruns_builtins::{
    BudgetingCapability, MessageMetadataCapability, SelfBudgetCapability,
    StatelessTodoListCapability,
};
use everruns_core::{Capability, SystemPromptContext};
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
async fn stateless_todo_list_prompt_within_budget() {
    assert_contribution_under(&StatelessTodoListCapability, 450).await;
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
