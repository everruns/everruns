// Memory v2 routing against PostgreSQL (knowledge/runtime-resources/memory.md):
// which stores a session gets from its `agent_memory` config, who owns them,
// what is read-only, what entry files are refused, and that every change is
// kept as a version.

use super::*;
use crate::domains::memory::entries::visible_to;
use crate::storage::{
    CreateAgentCapabilityRow, CreateAgentRow, CreateHarnessRow, SessionRow, StorageBackend,
};
use everruns_contracts::typed_id::AgentId;
use everruns_core::DEFAULT_ORG_ID;

async fn agent(db: &Arc<StorageBackend>, name: &str, memory: Option<serde_json::Value>) -> AgentId {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                localizations: Default::default(),
                name: format!("{name}-harness"),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let row = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                completion_check: Default::default(),
                localizations: Default::default(),
                public_id: AgentId::new().to_string(),
                name: name.to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: String::new(),
                default_model_id: None,
                harness_id: harness.id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                max_output_tokens: None,
                communication: Default::default(),
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    if let Some(config) = memory {
        db.add_agent_capability(CreateAgentCapabilityRow {
            agent_id: row.id,
            capability_id: AGENT_MEMORY_CAPABILITY_ID.into(),
            position: 0,
            config,
        })
        .await
        .unwrap();
    }
    row.id
}

async fn session(
    db: &Arc<StorageBackend>,
    agent: AgentId,
    parent: Option<&SessionRow>,
) -> SessionRow {
    let mut row = crate::storage::test_database::test_session_row(DEFAULT_ORG_ID);
    row.agent_id = Some(agent);
    row.parent_session_id = parent.map(|parent| parent.id);
    row.budget_root_session_id = parent.map(|parent| parent.id);
    db.create_session(row).await.unwrap()
}

const DECISION: &[u8] =
    b"---\nkind: decision\nsubject: everruns#5528\n---\nAllowlist @a2ui/web_core\n\n> allow it\n";

#[tokio::test]
async fn stores_follow_the_config_and_keep_every_version() {
    let db = Arc::new(StorageBackend::test_database());
    let coordinator = agent(
        &db,
        "Housekeeping",
        Some(serde_json::json!({
            "attach": [{"memory": "everruns-maintenance", "mode": "read_write"}]
        })),
    )
    .await;
    let worker = agent(&db, "Worker", Some(serde_json::json!({"org": "off"}))).await;
    let lead = session(&db, coordinator, None).await;
    let thread = session(&db, worker, Some(&lead)).await;
    let router = MemoryMountRouter::new(db.clone());

    // The coordinator: its own memory, the org's (read only), the named one.
    let (agent_mount, inner) = router
        .route(lead.id.uuid(), "/memory/agent/decisions/everruns-5528.md")
        .await
        .expect("agent memory is mounted");
    router
        .write_file(&agent_mount, &inner, DECISION.to_vec())
        .await
        .unwrap();
    let (org_mount, inner) = router
        .route(lead.id.uuid(), "/memory/org/rules/x.md")
        .await
        .expect("org memory is mounted");
    assert!(org_mount.readonly);
    assert!(
        router
            .write_file(&org_mount, &inner, DECISION.to_vec())
            .await
            .is_err()
    );
    let (named, _) = router
        .route(lead.id.uuid(), "/memory/everruns-maintenance/notes.md")
        .await
        .expect("an attached named memory is created and mounted");
    assert!(!named.readonly);
    // Without a parent there is no /memory/parent.
    assert!(
        router
            .route(lead.id.uuid(), "/memory/parent/x.md")
            .await
            .is_none()
    );

    // The worker: the coordinator's team memory, read-write, and its parent's
    // agent memory, read only; org switched off.
    let (team, inner) = router
        .route(thread.id.uuid(), "/memory/team/blocked/everruns-5528.md")
        .await
        .expect("team memory is mounted for a thread");
    assert!(!team.readonly);
    router
        .write_file(
            &team,
            &inner,
            b"---\nkind: blocked\nsubject: everruns#5528\n---\nWaiting on Mike\n".to_vec(),
        )
        .await
        .unwrap();
    let team_row = db
        .get_memory_by_id(DEFAULT_ORG_ID, team.memory_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(team_row.scope, "team");
    assert_eq!(team_row.owner_agent_id, Some(coordinator));
    let (parent, inner) = router
        .route(
            thread.id.uuid(),
            "/memory/parent/decisions/everruns-5528.md",
        )
        .await
        .expect("the parent's agent memory is mounted");
    assert!(parent.readonly);
    assert_eq!(parent.memory_id, agent_mount.memory_id);
    assert_eq!(
        router.read_file(&parent, &inner).await.unwrap().unwrap(),
        DECISION
    );
    assert!(
        router
            .route(thread.id.uuid(), "/memory/org/x.md")
            .await
            .is_none()
    );

    // An entry file that would not parse is refused; a free-form file is not.
    let (mount, inner) = router
        .route(lead.id.uuid(), "/memory/agent/decisions/broken.md")
        .await
        .unwrap();
    let refused = router
        .write_file(&mount, &inner, b"no header".to_vec())
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("decisions/broken.md"),
        "{refused}"
    );
    router
        .write_file(&mount, "/runbook.md", b"no header is fine here".to_vec())
        .await
        .unwrap();

    // Every change is kept, deletes included.
    router
        .write_file(
            &agent_mount,
            "/decisions/everruns-5528.md",
            b"---\nkind: decision\nsubject: everruns#5528\n---\nDeny it\n".to_vec(),
        )
        .await
        .unwrap();
    assert!(
        router
            .delete(&agent_mount, "/decisions/everruns-5528.md", false)
            .await
            .unwrap()
    );
    let versions = db
        .list_memory_file_versions(
            agent_mount.memory_id,
            Some("/decisions/everruns-5528.md"),
            10,
        )
        .await
        .unwrap();
    let actions: Vec<_> = versions.iter().map(|v| v.action.as_str()).collect();
    assert_eq!(actions, ["delete", "write", "write"]);
    assert_eq!(versions[2].content.as_deref(), Some(DECISION));
}

#[tokio::test]
async fn an_agent_without_the_capability_keeps_only_legacy_mounts() {
    let db = Arc::new(StorageBackend::test_database());
    let plain = agent(&db, "Plain", None).await;
    let row = session(&db, plain, None).await;
    let router = MemoryMountRouter::new(db.clone());
    assert!(
        router
            .route(row.id.uuid(), "/memory/agent/x.md")
            .await
            .is_some()
    );
    assert!(
        router
            .route(row.id.uuid(), "/memory/org/x.md")
            .await
            .is_none()
    );
    assert!(
        router
            .route(row.id.uuid(), "/memory/team/x.md")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_private_memory_is_listed_only_for_its_person() {
    let db = Arc::new(StorageBackend::test_database());
    let person = uuid::Uuid::now_v7();
    let mut row = db
        .list_memory_stores(DEFAULT_ORG_ID)
        .await
        .unwrap()
        .into_iter()
        .next();
    if row.is_none() {
        ensure_shared_memory(&db, DEFAULT_ORG_ID, ORG_MEMORY_NAME)
            .await
            .unwrap();
        row = db
            .list_memory_stores(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .into_iter()
            .next();
    }
    let mut private = row.unwrap();
    private.scope = "user".into();
    private.owner_user_id = Some(person);
    assert!(visible_to(&private, Some(person)));
    assert!(!visible_to(&private, Some(uuid::Uuid::now_v7())));
    assert!(!visible_to(&private, None));
    private.scope = "org".into();
    assert!(visible_to(&private, None));
}
