use super::*;

#[tokio::test]
async fn installing_skills_keeps_system_prompt_byte_identical() {
    let cap = SkillsCapability;
    let store = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let ctx = SystemPromptContext {
        file_store: Some(store.clone()),
        ..SystemPromptContext::without_file_store(session_id)
    };
    let before = cap.system_prompt_contribution(&ctx).await.unwrap();
    store.add_file(
        session_id,
        "/.agents/skills/new-skill/SKILL.md",
        &valid_skill_md("new-skill", "Newly installed skill"),
    );
    assert_eq!(before, cap.system_prompt_contribution(&ctx).await.unwrap());
    let installed = cap
        .conversation_context_update_contribution(&ctx)
        .await
        .unwrap();
    assert!(installed.contains("new-skill"));
    assert!(installed.contains("Newly installed skill"));
    store.add_file(
        session_id,
        "/.agents/skills/new-skill/SKILL.md",
        &valid_skill_md("new-skill", "Edited description"),
    );
    assert!(
        cap.conversation_context_update_contribution(&ctx)
            .await
            .unwrap()
            .contains("Edited description")
    );
    assert_eq!(before, cap.system_prompt_contribution(&ctx).await.unwrap());
    // Another session cannot inherit this session's inventory.
    let other = SystemPromptContext {
        file_store: Some(store.clone()),
        ..SystemPromptContext::without_file_store(SessionId::new())
    };
    assert_eq!(
        cap.conversation_context_update_contribution(&other)
            .await
            .as_deref(),
        Some(EMPTY_SKILL_INVENTORY)
    );
    store
        .files
        .lock()
        .unwrap()
        .remove(&(session_id, "/.agents/skills/new-skill/SKILL.md".to_string()));
    assert_eq!(
        cap.conversation_context_update_contribution(&ctx)
            .await
            .as_deref(),
        Some(EMPTY_SKILL_INVENTORY)
    );
    assert_eq!(before, cap.system_prompt_contribution(&ctx).await.unwrap());
    store.list_unavailable.store(true, Ordering::SeqCst);
    assert_eq!(
        cap.conversation_context_update_contribution(&ctx)
            .await
            .as_deref(),
        Some(EMPTY_SKILL_INVENTORY)
    );
}
