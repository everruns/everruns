use super::*;

#[tokio::test]
async fn test_search_agents_multi_word_all_must_match() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Customer Support Bot", None).await;
    create_test_agent(&db, "Customer Feedback Analyzer", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("customer bot"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Customer Support Bot");
}

#[tokio::test]
async fn test_search_agents_matches_description() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Helper", Some("Handles billing inquiries")).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("billing"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Helper");
}

#[tokio::test]
async fn test_search_agents_cross_field_match() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Daytona Coder", Some("cloud sandbox agent")).await;

    // "daytona" in name, "sandbox" in description → both must match
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("daytona sandbox"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_no_match() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Customer Support Bot", None).await;

    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("zzz_nonexistent"),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_agents_poem_does_not_crash() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Alpha", None).await;

    // A full poem pasted into search — should not crash or hang
    let poem = "Roses are red, violets are blue, \
                    sugar is sweet, and so are you. \
                    The sky is wide, the ocean deep, \
                    these memories I shall forever keep. \
                    Through winding roads and starlit nights, \
                    we chase our dreams to greater heights.";
    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some(poem), false, default_pagination())
        .await
        .unwrap();
    // No agent should match a poem
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_agents_poem_token_cap() {
    let db = StorageBackend::test_database();
    // Agent whose name contains many words from the poem
    create_test_agent(&db, "roses are red violets are blue sugar is sweet", None).await;

    // Query with >MAX_SEARCH_TOKENS words — only first 8 tokens used
    let long_query = "roses are red violets are blue sugar is sweet and so are you forever";
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some(long_query),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    // First 8 tokens: "roses are red violets are blue sugar is"
    // All present in agent name → should match
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_special_characters() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Agent v2.0 (beta)", None).await;
    create_test_agent(&db, "my-agent_v1", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("v2.0"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Agent v2.0 (beta)");
}

#[tokio::test]
async fn test_search_agents_unicode() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "日本語エージェント", Some("テスト用")).await;
    create_test_agent(&db, "English Agent", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("日本語"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "日本語エージェント");
}

#[tokio::test]
async fn test_search_agents_emoji() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "🤖 Robot Helper", None).await;
    create_test_agent(&db, "Normal Agent", None).await;

    let (results, _total) = db
        .list_agents(DEFAULT_ORG_ID, Some("🤖"), false, default_pagination())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_agents_whitespace_normalization() {
    let db = StorageBackend::test_database();
    create_test_agent(&db, "Customer Support Bot", None).await;

    // Extra spaces, tabs, etc.
    let (results, _total) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some("  customer   bot  "),
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_sessions_by_title() {
    let db = StorageBackend::test_database();
    let agent = create_test_agent(&db, "Agent", None).await;

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        agent_id: Some(agent.id),
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        title: Some("Debug production memory leak".to_string()),
        ..Default::default()
    })
    .await
    .unwrap();

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        agent_id: Some(agent.id),
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        title: Some("Refactor auth module".to_string()),
        ..Default::default()
    })
    .await
    .unwrap();

    let pagination = crate::api::common::Pagination::new(0, 20);
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                search: Some("memory leak".to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
    assert!(results[0].title.as_ref().unwrap().contains("memory leak"));
}

#[tokio::test]
async fn test_search_sessions_with_agent_filter() {
    let db = StorageBackend::test_database();
    let agent1 = create_test_agent(&db, "Agent1", None).await;
    let agent2 = create_test_agent(&db, "Agent2", None).await;

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        agent_id: Some(agent1.id),
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        title: Some("Shared keyword session".to_string()),
        ..Default::default()
    })
    .await
    .unwrap();

    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        agent_id: Some(agent2.id),
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        title: Some("Shared keyword session".to_string()),
        ..Default::default()
    })
    .await
    .unwrap();

    let pagination = crate::api::common::Pagination::new(0, 20);
    // Search + agent filter combined
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent1.id),
                search: Some("shared keyword".to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_search_sessions_poem_input() {
    let db = StorageBackend::test_database();

    let pagination = crate::api::common::Pagination::new(0, 20);
    let poem = "Shall I compare thee to a summer's day? \
                    Thou art more lovely and more temperate. \
                    Rough winds do shake the darling buds of May, \
                    And summer's lease hath all too short a date.";
    let (results, total) = db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters {
                search: Some(poem.to_string()),
                ..Default::default()
            },
            pagination,
        )
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_skills() {
    let db = StorageBackend::test_database();

    db.create_skill(
        DEFAULT_ORG_ID,
        CreateSkillRow {
            public_id: SkillId::new().to_string(),
            name: "Web Scraper".to_string(),
            description: "Scrapes web pages for data".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("*".to_string()),
            metadata: serde_json::json!({}),
            allowed_tools: None,
            instructions: "scrape it".to_string(),
            source_type: "markdown".to_string(),
            archive_data: None,
            version: "1.0.0".to_string(),
        },
    )
    .await
    .unwrap();

    db.create_skill(
        DEFAULT_ORG_ID,
        CreateSkillRow {
            public_id: SkillId::new().to_string(),
            name: "Code Formatter".to_string(),
            description: "Formats code using prettier".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("*".to_string()),
            metadata: serde_json::json!({}),
            allowed_tools: None,
            instructions: "format it".to_string(),
            source_type: "markdown".to_string(),
            archive_data: None,
            version: "1.0.0".to_string(),
        },
    )
    .await
    .unwrap();

    let results = db
        .list_skills(DEFAULT_ORG_ID, Some("scraper"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Web Scraper");

    // Cross-field: "code prettier"
    let results = db
        .list_skills(DEFAULT_ORG_ID, Some("code prettier"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Code Formatter");
}

#[tokio::test]
async fn test_search_apps() {
    let db = StorageBackend::test_database();
    let agent = create_test_agent(&db, "Agent", None).await;
    // The fixture harness.
    let harness_id = Uuid::nil();

    db.create_app(
        DEFAULT_ORG_ID,
        CreateAppRow {
            public_id: format!("app_{}", uuid::Uuid::now_v7().simple()),
            name: "Slack Bot".to_string(),
            description: Some("Slack integration for support".to_string()),
            harness_id,
            agent_id: Some(agent.id.into()),
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            channel_type: Some("slack".to_string()),
            channel_config: serde_json::json!({}),
            channel_config_encrypted: None,
        },
    )
    .await
    .unwrap();

    db.create_app(
        DEFAULT_ORG_ID,
        CreateAppRow {
            public_id: format!("app_{}", uuid::Uuid::now_v7().simple()),
            name: "Web Widget".to_string(),
            description: Some("Embeddable chat widget".to_string()),
            harness_id,
            agent_id: Some(agent.id.into()),
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            channel_type: Some("ag_ui".to_string()),
            channel_config: serde_json::json!({}),
            channel_config_encrypted: None,
        },
    )
    .await
    .unwrap();

    let results = db
        .list_apps(DEFAULT_ORG_ID, Some("slack"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Slack Bot");

    // Multi-word across name + description
    let results = db
        .list_apps(DEFAULT_ORG_ID, Some("widget chat"), false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Web Widget");
}

// ─── MessageFilter::Search tests (EVE-87) ───

#[tokio::test]
async fn test_search_filter_matches_content_field() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("Rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    // Should match: "Tell me about Rust programming", "Rust is a systems language",
    // "Here is information about Rust"
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn test_search_filter_case_insensitive() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 3);

    // Also uppercase
    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("RUST".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn test_search_filter_no_match() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    let query = MessageQuery::new(session_id)
        .with_filter(MessageFilter::Search("nonexistent_xyz".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_search_filter_skips_events_without_content() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    // "abc123" is in turn_id, not content — should not match
    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("abc123".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_search_filter_partial_match() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    let query =
        MessageQuery::new(session_id).with_filter(MessageFilter::Search("great".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "output.message.completed");
}

#[tokio::test]
async fn test_search_filter_combined_with_event_type() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    // Search for "Rust" but only in input.message events
    let query = MessageQuery::new(session_id)
        .with_filter(MessageFilter::EventTypes(vec!["input.message".to_string()]))
        .with_filter(MessageFilter::Search("Rust".to_string()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "input.message");
}

#[tokio::test]
async fn test_list_sessions_waiting_tool_results_before() {
    let db = StorageBackend::test_database();
    let now = Utc::now();
    let cutoff = now - chrono::Duration::minutes(5);

    // Create 3 sessions: one waiting+old, one waiting+recent, one active+old
    let s1 = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            capabilities: serde_json::json!({}),
            ..Default::default()
        })
        .await
        .unwrap();
    let s2 = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            capabilities: serde_json::json!({}),
            ..Default::default()
        })
        .await
        .unwrap();
    let s3 = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            capabilities: serde_json::json!({}),
            ..Default::default()
        })
        .await
        .unwrap();

    // Manually set statuses and updated_at
    for (id, status, age) in [
        (s1.id, "waiting_for_tool_results", 10), // old, should match
        (s2.id, "waiting_for_tool_results", 1),  // recent, should NOT match
        (s3.id, "active", 10),                   // old but active, should NOT match
    ] {
        set_session_status_and_updated_at(&db, id, status, now - chrono::Duration::minutes(age))
            .await;
    }

    let result = db
        .list_sessions_waiting_tool_results_before(cutoff)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, s1.id);
    assert_eq!(result[0].1, DEFAULT_ORG_ID);
}

#[tokio::test]
async fn test_search_filter_empty_string_matches_nothing() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    // Search is full-text (`plainto_tsquery`), and an empty query has no
    // terms to match, so it selects nothing rather than everything.
    let query = MessageQuery::new(session_id).with_filter(MessageFilter::Search(String::new()));

    let events = db.list_message_events_filtered(&query).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_filtered_message_limit_keeps_latest_events_chronological() {
    let db = StorageBackend::test_database();
    let session_id = create_session_with_content_events(&db).await;

    let query = MessageQuery::new(session_id).with_limit(2);

    let events = db.list_message_events_filtered(&query).await.unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "tool.completed");
    assert_eq!(events[1].event_type, "output.message.completed");
}

#[tokio::test]
async fn test_session_system_prompt_and_initial_files_round_trip() {
    let db = StorageBackend::test_database();

    let initial_files = serde_json::json!([
        {"path": "/workspace/hello.txt", "content": "hello", "encoding": "text"},
        {"path": "/workspace/config.json", "content": "{}", "encoding": "text"}
    ]);

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            title: Some("Override Test".to_string()),
            system_prompt: Some("You are a session-level override".to_string()),
            initial_files: initial_files.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        session.system_prompt,
        Some("You are a session-level override".to_string())
    );
    assert_eq!(session.initial_files, initial_files);

    // Verify round-trip via get
    let fetched = db
        .get_session(DEFAULT_ORG_ID, session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fetched.system_prompt,
        Some("You are a session-level override".to_string())
    );
    assert_eq!(fetched.initial_files, initial_files);
}

#[tokio::test]
async fn test_session_system_prompt_defaults_to_none() {
    let db = StorageBackend::test_database();

    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(session.system_prompt, None);
    assert_eq!(session.initial_files, serde_json::json!([]));
}

#[tokio::test]
async fn test_delete_user_account() {
    let db = StorageBackend::test_database();

    let user = db
        .create_user(CreateUserRow {
            email: "delete@example.com".to_string(),
            name: "Delete Me".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Create personal access token for user
    db.create_personal_access_token(CreatePersonalAccessTokenRow {
        user_id: user.id,
        name: "test-token".to_string(),
        token_hash: "hash123".to_string(),
        token_prefix: "evr_pat_".to_string(),
        scopes: vec![],
        expires_at: None,
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();

    // Create refresh token for user
    db.create_refresh_token(CreateRefreshTokenRow {
        user_id: user.id,
        token_hash: "refresh_hash".to_string(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    })
    .await
    .unwrap();

    // Add user to default org
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .unwrap();

    // Verify user exists with related data
    assert!(db.get_user(user.id).await.unwrap().is_some());
    assert_eq!(
        db.list_personal_access_tokens_for_user(user.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        db.is_organization_member(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
    );

    // Delete account
    let deleted = db.delete_user_account(user.id).await.unwrap();
    assert!(deleted);

    // Verify cascading delete of all user data
    assert!(db.get_user(user.id).await.unwrap().is_none());
    assert_eq!(
        db.list_personal_access_tokens_for_user(user.id)
            .await
            .unwrap()
            .len(),
        0
    );
    assert!(
        !db.is_organization_member(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
    );

    // Deleting non-existent user returns false
    let deleted_again = db.delete_user_account(user.id).await.unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn test_export_user_data() {
    let db = StorageBackend::test_database();

    let user = db
        .create_user(CreateUserRow {
            email: "export@example.com".to_string(),
            name: "Export User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Create personal access token
    db.create_personal_access_token(CreatePersonalAccessTokenRow {
        user_id: user.id,
        name: "my-token".to_string(),
        token_hash: "hash456".to_string(),
        token_prefix: "evr_pat_".to_string(),
        scopes: vec!["read".to_string()],
        expires_at: None,
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();

    // Export data
    let export = db.export_user_data(user.id).await.unwrap().unwrap();

    assert_eq!(export["user"]["email"], "export@example.com");
    assert_eq!(export["user"]["name"], "Export User");
    assert_eq!(
        export["personal_access_tokens"].as_array().unwrap().len(),
        1
    );
    assert_eq!(export["personal_access_tokens"][0]["name"], "my-token");
    // Verify no sensitive data is exported
    assert!(
        export["personal_access_tokens"][0]
            .get("token_hash")
            .is_none()
    );
    assert!(export.get("exported_at").is_some());

    // Non-existent user returns None
    let missing = db.export_user_data(uuid::Uuid::now_v7()).await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_user_preferences_crud_and_isolation() {
    let db = StorageBackend::test_database();
    let user_a = db.create_test_user(uuid::Uuid::now_v7()).await;
    let user_b = db.create_test_user(uuid::Uuid::now_v7()).await;

    // Missing key reads as None.
    assert!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .is_none()
    );

    // Set creates the row.
    let created = db
        .set_user_preference(user_a, "theme", "\"dark\"", 100)
        .await
        .unwrap();
    assert_eq!(created.key, "theme");
    assert_eq!(created.value, "\"dark\"");

    // Set again upserts (updates value, keeps identity, no duplicate row).
    let updated = db
        .set_user_preference(user_a, "theme", "\"light\"", 100)
        .await
        .unwrap();
    assert_eq!(updated.id, created.id, "upsert must reuse the same row");
    assert_eq!(updated.value, "\"light\"");
    assert_eq!(
        db.list_user_preferences(user_a, 100).await.unwrap().len(),
        1
    );

    // Preferences are isolated per user.
    db.set_user_preference(user_b, "theme", "\"system\"", 100)
        .await
        .unwrap();
    let quota_error = db
        .set_user_preference(user_b, "locale", "\"en\"", 1)
        .await
        .unwrap_err();
    assert_eq!(
        quota_error.to_string(),
        super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED
    );
    assert_eq!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .unwrap()
            .value,
        "\"light\""
    );
    assert_eq!(
        db.list_user_preferences(user_b, 100).await.unwrap().len(),
        1
    );

    // Delete removes only the targeted key and reports whether a row was hit.
    assert!(db.delete_user_preference(user_a, "theme").await.unwrap());
    assert!(!db.delete_user_preference(user_a, "theme").await.unwrap());
    assert!(
        db.get_user_preference(user_a, "theme")
            .await
            .unwrap()
            .is_none()
    );
    // user_b is unaffected by user_a's delete.
    assert_eq!(
        db.list_user_preferences(user_b, 100).await.unwrap().len(),
        1
    );
}

// OAuth linking must preserve password authentication for an existing email.
#[tokio::test]
async fn link_oauth_identity_attaches_provider_and_preserves_password() {
    let db = StorageBackend::test_database();

    let email = format!("linker-{}@example.com", Uuid::now_v7());
    let user = db
        .create_user(CreateUserRow {
            email: email.clone(),
            name: "Linker".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some("argon2-hash".to_string()),
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Before linking, the Google identity resolves to nobody.
    assert!(
        db.get_user_by_oauth("google", "google-sub-1")
            .await
            .unwrap()
            .is_none()
    );

    let linked = db
        .link_oauth_identity(user.id, "google", "google-sub-1")
        .await
        .unwrap()
        .expect("existing user linked");
    assert_eq!(linked.id, user.id);

    // Google login now resolves to the same account.
    let by_oauth = db
        .get_user_by_oauth("google", "google-sub-1")
        .await
        .unwrap()
        .expect("oauth lookup resolves to linked account");
    assert_eq!(by_oauth.id, user.id);

    // Password auth is preserved: hash intact and email lookup still works, so
    // password login and password reset keep functioning for the linked user.
    assert_eq!(by_oauth.password_hash.as_deref(), Some("argon2-hash"));
    let by_email = db.get_user_by_email(&email).await.unwrap().unwrap();
    assert_eq!(by_email.id, user.id);
    assert_eq!(by_email.password_hash.as_deref(), Some("argon2-hash"));

    // Linking a non-existent user is a no-op (None), not an error.
    assert!(
        db.link_oauth_identity(Uuid::now_v7(), "google", "x")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn link_oauth_identity_preserves_other_provider_logins() {
    let db = StorageBackend::test_database();
    let user = db
        .create_user(CreateUserRow {
            email: format!("multi-oauth-{}@example.com", Uuid::now_v7()),
            name: "Multi OAuth".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("github".to_string()),
            auth_provider_id: Some("github-user-1".to_string()),
            external_id: None,
        })
        .await
        .unwrap();

    let linked = db
        .link_oauth_identity(user.id, "google", "google-user-1")
        .await
        .unwrap()
        .expect("second provider linked");
    assert_eq!(linked.id, user.id);

    for (provider, provider_id) in [("github", "github-user-1"), ("google", "google-user-1")] {
        assert_eq!(
            db.get_user_by_oauth(provider, provider_id)
                .await
                .unwrap()
                .expect("provider login resolves")
                .id,
            user.id
        );
    }
}

#[tokio::test]
async fn link_oauth_identity_does_not_replace_existing_provider_subject() {
    let db = StorageBackend::test_database();
    let user = db
        .create_user(CreateUserRow {
            email: format!("provider-lock-{}@example.com", Uuid::now_v7()),
            name: "Provider Lock".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("google".to_string()),
            auth_provider_id: Some("google-original".to_string()),
            external_id: None,
        })
        .await
        .unwrap();
    let other_user = db
        .create_user(CreateUserRow {
            email: format!("provider-lock-other-{}@example.com", Uuid::now_v7()),
            name: "Other User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    assert!(
        db.link_oauth_identity(user.id, "google", "google-replacement")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.get_user_by_oauth("google", "google-original")
            .await
            .unwrap()
            .expect("original identity remains")
            .id,
        user.id
    );
    assert!(
        db.get_user_by_oauth("google", "google-replacement")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.link_oauth_identity(other_user.id, "google", "google-original")
            .await
            .unwrap()
            .is_none()
    );
}

// EVE-704: user email is case-insensitive across login and OAuth linking.
#[tokio::test]
async fn test_user_email_is_case_insensitive_identity() {
    let db = StorageBackend::test_database();

    // Registered with mixed case and stray surrounding whitespace.
    let created = db
        .create_user(CreateUserRow {
            email: "  John.Doe@Example.COM ".to_string(),
            name: "John".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some("argon2-hash".to_string()),
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();

    // Stored in canonical (trim + lowercase) form.
    assert_eq!(created.email, "john.doe@example.com");

    // Every casing of the same mailbox resolves to the one account — this is the
    // register pre-check (duplicate signup blocked), login, and OAuth-linking
    // lookup path in `auth::routes`.
    for lookup in [
        "john.doe@example.com",
        "John.Doe@Example.COM",
        "JOHN.DOE@EXAMPLE.COM",
        "  john.doe@example.com  ",
    ] {
        let found = db
            .get_user_by_email(lookup)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("email lookup {lookup:?} must resolve to the account"));
        assert_eq!(
            found.id, created.id,
            "lookup {lookup:?} resolved to wrong account"
        );
    }
}

#[test]
fn test_normalize_email_trims_and_lowercases() {
    assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
    assert_eq!(normalize_email("alice@example.com"), "alice@example.com");
    assert_eq!(normalize_email("\tBob@X.io\n"), "bob@x.io");
}

// Agent trigger round-trips (EVE-757)

#[tokio::test]
async fn test_agent_trigger_create_get_list_update_delete_round_trip() {
    let db = StorageBackend::test_database();
    let agent_id = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);

    let created = db
        .create_agent_trigger(schedule_trigger_input(agent_id))
        .await
        .unwrap();
    assert_eq!(created.status, "active");
    assert_eq!(created.trigger_type, "schedule");
    assert!(created.enabled);
    assert_eq!(created.agent_id, agent_id);

    // Config round-trips into the typed core accessor.
    let trigger =
        crate::domains::agent_triggers::queries::row_to_trigger(created.clone(), agent_id, None);
    let schedule = trigger.schedule_config().unwrap();
    assert_eq!(schedule.cron_expression, "0 0 * * * *");
    assert_eq!(schedule.message, "hello");

    let fetched = db
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .unwrap()
        .expect("trigger exists");
    assert_eq!(fetched.id, created.id);

    // Cross-org isolation.
    assert!(
        db.get_agent_trigger(999, created.id)
            .await
            .unwrap()
            .is_none()
    );

    let listed = db
        .list_agent_triggers(DEFAULT_ORG_ID, None, false)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);

    // Update
    let updated = db
        .update_agent_trigger(
            DEFAULT_ORG_ID,
            created.id,
            UpdateAgentTrigger {
                enabled: Some(false),
                config: Some(serde_json::json!({
                    "cron_expression": "0 5 * * * *",
                    "message": "updated",
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("update returns row");
    assert!(!updated.enabled);
    assert_eq!(updated.config["message"], serde_json::json!("updated"));

    // Soft delete (archive)
    assert!(
        db.delete_agent_trigger(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap()
    );
    let after_delete = db
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .unwrap()
        .expect("row still present after soft delete");
    assert_eq!(after_delete.status, "archived");
    assert!(after_delete.archived_at.is_some());

    // Archived rows are excluded unless include_archived.
    assert!(
        db.list_agent_triggers(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.list_agent_triggers(DEFAULT_ORG_ID, None, true)
            .await
            .unwrap()
            .len(),
        1
    );

    // Second delete is a no-op (already archived, not active).
    assert!(
        !db.delete_agent_trigger(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_agent_trigger_set_durable_schedule_id() {
    let db = StorageBackend::test_database();
    let created = db
        .create_agent_trigger(schedule_trigger_input(AgentId::from_uuid(
            db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await,
        )))
        .await
        .unwrap();
    assert!(created.durable_schedule_id.is_none());

    let schedule_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO durable_schedules (id, name, cron_expression, target_type, target_name) \
         VALUES ($1, $2, '0 0 * * * * *', 'workflow', 'test')",
    )
    .bind(schedule_id)
    .bind(format!("test-schedule-{}", schedule_id.simple()))
    .execute(db.database().pool())
    .await
    .unwrap();
    let bound = db
        .set_agent_trigger_durable_schedule_id(DEFAULT_ORG_ID, created.id, Some(schedule_id))
        .await
        .unwrap()
        .expect("bind returns row");
    assert_eq!(bound.durable_schedule_id, Some(schedule_id));

    // Clearing the binding works too.
    let cleared = db
        .set_agent_trigger_durable_schedule_id(DEFAULT_ORG_ID, created.id, None)
        .await
        .unwrap()
        .expect("clear returns row");
    assert!(cleared.durable_schedule_id.is_none());
}

#[tokio::test]
async fn test_agent_trigger_list_filters_by_agent() {
    let db = StorageBackend::test_database();
    let agent_a = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
    let agent_b = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);

    db.create_agent_trigger(schedule_trigger_input(agent_a))
        .await
        .unwrap();
    db.create_agent_trigger(schedule_trigger_input(agent_a))
        .await
        .unwrap();
    db.create_agent_trigger(schedule_trigger_input(agent_b))
        .await
        .unwrap();

    let for_a = db
        .list_agent_triggers(DEFAULT_ORG_ID, Some(agent_a), false)
        .await
        .unwrap();
    assert_eq!(for_a.len(), 2);
    assert!(for_a.iter().all(|t| t.agent_id == agent_a));

    let for_b = db
        .list_agent_triggers(DEFAULT_ORG_ID, Some(agent_b), false)
        .await
        .unwrap();
    assert_eq!(for_b.len(), 1);

    let all = db
        .list_agent_triggers(DEFAULT_ORG_ID, None, false)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
}
