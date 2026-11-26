use super::*;
use crate::host::InMemorySessionFileStore;

async fn session_with(stores: &[&str]) -> (InMemorySessionFileStore, SessionId) {
    let fs = InMemorySessionFileStore::new();
    let session = SessionId::new();
    for store in stores {
        fs.create_directory(session, &format!("/memory/{store}"))
            .await
            .unwrap();
    }
    (fs, session)
}

fn decision(subject: &str, statement: &str) -> NewMemoryEntry {
    NewMemoryEntry {
        kind: MemoryEntryKind::Decision,
        subject: Some(subject.into()),
        statement: statement.into(),
        quote: Some("allow it".into()),
        source: None,
        occurred_at: None,
    }
}

#[tokio::test]
async fn stores_are_config_switches_intersected_with_mounts() {
    let (fs, session) = session_with(&["agent", "team", "org", "everruns-maintenance"]).await;
    let config = AgentMemoryConfig::from_value(&json!({
        "org": "off",
        "attach": [{"memory": "everruns-maintenance", "mode": "read"}]
    }))
    .unwrap();
    let engine = MemoryEngine::new(&fs, session, &config);
    let stores: Vec<_> = engine
        .stores()
        .await
        .into_iter()
        .map(|s| (s.key.label(), s.access))
        .collect();
    // No user or parent mounted, org switched off.
    assert_eq!(
        stores,
        vec![
            ("agent".into(), MemoryAccess::ReadWrite),
            ("team".into(), MemoryAccess::ReadWrite),
            ("everruns-maintenance".into(), MemoryAccess::Read),
        ]
    );
}

#[tokio::test]
async fn remember_replaces_same_subject_and_recall_finds_it() {
    let (fs, session) = session_with(&["agent", "team"]).await;
    let config = AgentMemoryConfig::default();
    let engine = MemoryEngine::new(&fs, session, &config).with_actor("agent:coordinator");
    let team = MemoryStoreKey::scope(MemoryStoreScope::Team);

    let first = engine
        .remember(
            &team,
            decision("everruns#5528", "Deny @a2ui/web_core"),
            false,
        )
        .await
        .unwrap();
    assert!(first.replaced.is_none());
    assert_eq!(
        first.entry.session_path(),
        "/memory/team/decisions/everruns-5528.md"
    );
    let second = engine
        .remember(
            &team,
            decision("Everruns#5528", "Allowlist @a2ui/web_core"),
            false,
        )
        .await
        .unwrap();
    assert_eq!(second.replaced.unwrap().statement, "Deny @a2ui/web_core");

    // The file is plain Markdown any file tool can read.
    let file = fs
        .read_file(session, "/memory/team/decisions/everruns-5528.md")
        .await
        .unwrap()
        .unwrap();
    let text = file.content.unwrap();
    assert!(text.contains("recorded_by: agent:coordinator"));
    assert!(text.contains("> allow it"));

    let hits = engine
        .recall(&RecallQuery {
            text: "what about 5528?".into(),
            include_files: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let RecallHit::Entry { entry, .. } = &hits[0] else {
        panic!("expected an entry");
    };
    assert_eq!(entry.statement, "Allowlist @a2ui/web_core");
}

#[tokio::test]
async fn read_only_and_missing_stores_refuse_writes() {
    let (fs, session) = session_with(&["agent", "org"]).await;
    let config = AgentMemoryConfig::default();
    let engine = MemoryEngine::new(&fs, session, &config);
    let org = engine
        .remember(
            &MemoryStoreKey::scope(MemoryStoreScope::Org),
            decision("x", "y"),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(org, MemoryError::ReadOnly(_)), "{org}");
    let team = engine
        .remember(
            &MemoryStoreKey::scope(MemoryStoreScope::Team),
            decision("x", "y"),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(team, MemoryError::Unavailable(_)), "{team}");
    let forget = engine
        .forget("/memory/org/decisions/x.md")
        .await
        .unwrap_err();
    assert!(matches!(forget, MemoryError::ReadOnly(_)));
}

#[tokio::test]
async fn blocked_items_resolve_and_leave_the_card() {
    let (fs, session) = session_with(&["team"]).await;
    let config = AgentMemoryConfig::default();
    let engine = MemoryEngine::new(&fs, session, &config);
    let team = MemoryStoreKey::scope(MemoryStoreScope::Team);
    let blocked = engine
        .remember(
            &team,
            NewMemoryEntry {
                kind: MemoryEntryKind::Blocked,
                subject: Some("everruns#5528".into()),
                statement: "Waiting on Mike to allowlist the package".into(),
                quote: None,
                source: None,
                occurred_at: None,
            },
            false,
        )
        .await
        .unwrap();
    fs.write_file(
        session,
        "/memory/team/runbook.md",
        "# Runbook\nsteps",
        "text",
    )
    .await
    .unwrap();

    let card = engine.card().await.render().unwrap();
    assert!(card.contains("## team memory (read-write, files at /memory/team)"));
    assert!(card.contains("- blocked [everruns#5528]"));
    assert!(card.contains("/memory/team/runbook.md: Runbook"));

    let path = blocked.entry.session_path();
    // A decision cannot be resolved.
    engine
        .remember(&team, decision("merges", "squash"), false)
        .await
        .unwrap();
    assert!(
        engine
            .resolve("/memory/team/decisions/merges.md", None)
            .await
            .is_err()
    );

    let resolved = engine
        .resolve(&path, Some("allowlisted".into()))
        .await
        .unwrap();
    assert!(resolved.resolved_at.is_some());
    let card = engine.card().await.render().unwrap();
    assert!(!card.contains("blocked [everruns#5528]"));

    let closed = engine
        .recall(&RecallQuery {
            kinds: vec![MemoryEntryKind::Blocked],
            include_closed: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);

    engine.forget(&path).await.unwrap();
    assert!(engine.forget(&path).await.is_err());
}

#[tokio::test]
async fn proposals_wait_until_accepted() {
    let (fs, session) = session_with(&["org"]).await;
    let config = AgentMemoryConfig::from_value(&json!({"org": "read_write"})).unwrap();
    let engine = MemoryEngine::new(&fs, session, &config).with_actor("dreaming");
    let org = MemoryStoreKey::scope(MemoryStoreScope::Org);
    let proposal = engine
        .remember(&org, decision("merges", "squash only"), true)
        .await
        .unwrap();
    assert_eq!(
        proposal.entry.session_path(),
        "/memory/org/proposals/decisions/merges.md"
    );
    let card = engine.card().await.render().unwrap();
    assert!(!card.contains("squash only"));

    let accepted = engine.accept(&proposal.entry.session_path()).await.unwrap();
    assert_eq!(
        accepted.entry.session_path(),
        "/memory/org/decisions/merges.md"
    );
    assert!(
        fs.read_file(session, "/memory/org/proposals/decisions/merges.md")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        engine
            .card()
            .await
            .render()
            .unwrap()
            .contains("squash only")
    );
    assert!(engine.accept(&accepted.entry.session_path()).await.is_err());
}

#[tokio::test]
async fn recall_searches_free_files_and_skips_broken_entries() {
    let (fs, session) = session_with(&["agent"]).await;
    fs.write_file(
        session,
        "/memory/agent/notes.md",
        "a\nthe deploy key rotates monthly\nb",
        "text",
    )
    .await
    .unwrap();
    fs.write_file(
        session,
        "/memory/agent/decisions/broken.md",
        "no header deploy",
        "text",
    )
    .await
    .unwrap();
    let config = AgentMemoryConfig::default();
    let engine = MemoryEngine::new(&fs, session, &config);
    let hits = engine
        .recall(&RecallQuery {
            text: "deploy key".into(),
            include_files: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let RecallHit::File { file, .. } = &hits[0] else {
        panic!("expected a file");
    };
    assert_eq!(file.path, "/memory/agent/notes.md");
    assert!(file.excerpt.contains("rotates monthly"));
}

#[tokio::test]
async fn tools_and_card_run_through_the_capability() {
    let (fs, session) = session_with(&["agent", "team"]).await;
    let fs: Arc<dyn SessionFileSystem> = Arc::new(fs);
    let context = ToolContext::with_file_store(session, fs.clone());
    let config = json!({"user": "off"});
    let tools = AgentMemoryCapability.tools_with_config(&config);
    let names: Vec<_> = tools.iter().map(|t| t.name().to_string()).collect();
    assert_eq!(names, ["remember", "recall", "forget", "resolve", "accept"]);

    let remember = &tools[0];
    let result = remember
        .execute_with_context(
            json!({"store": "team", "kind": "rule", "subject": "merges", "statement": "Squash merge only", "quote": "always squash"}),
            &context,
        )
        .await;
    assert!(!result.is_error(), "{result:?}");
    let bad = remember
        .execute_with_context(
            json!({"kind": "decision", "statement": "no subject"}),
            &context,
        )
        .await;
    assert!(bad.is_error());

    let ctx = SystemPromptContext {
        session_id: session,
        locale: None,
        file_store: Some(fs),
        model: None,
        session_storage: None,
        tool_disclosure: Default::default(),
        tool_surface_plan: Default::default(),
    };
    let card = AgentMemoryCapability
        .conversation_context_update_contribution_with_config(&ctx, &config)
        .await
        .unwrap();
    assert!(card.contains("rule [merges]"));
    assert!(card.contains("## agent memory"));
    assert!(
        AgentMemoryCapability
            .conversation_context_update_contribution_with_config(&ctx, &json!({"card": false}))
            .await
            .is_none()
    );
    assert!(
        AgentMemoryCapability
            .validate_config(&json!({"team": "maybe"}))
            .is_err()
    );
}
