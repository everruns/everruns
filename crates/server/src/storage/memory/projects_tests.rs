//! Project boundary at the storage layer (knowledge/security/multitenancy.md,
//! "Projects (nested scope)"): agents are isolated by project, sessions follow
//! their agent, and names are unique per project.

use super::super::models::*;
use super::tests::{default_pagination, test_harness_id, test_session_input};
use super::*;
use everruns_core::DEFAULT_PROJECT_ID;

/// A minimal agent in `project_id`, for project-boundary tests.
fn project_agent_input(project_id: i64, name: &str) -> CreateAgentRow {
    CreateAgentRow {
        project_id,
        public_id: AgentId::new().to_string(),
        name: name.to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: serde_json::json!([]),
        system_prompt: "p".to_string(),
        default_model_id: None,
        harness_id: test_harness_id(),
        tags: vec![],
        initial_files: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        is_built_in: false,
    }
}

/// Agents are hard-isolated by project: list/resolve scoped to a project only
/// see that project's agents; `None` scope is org-wide (internal/worker paths).
#[tokio::test]
async fn test_agents_isolated_by_project() {
    let db = InMemoryDatabase::new();

    // Two projects in the default org (project 1 is the seeded default).
    let proj_a = DEFAULT_PROJECT_ID;
    let proj_b = db
        .create_project(CreateProjectRow {
            public_id: "proj_000000000000000000000000000000bb".to_string(),
            org_id: DEFAULT_ORG_ID,
            name: "Project B".to_string(),
            description: None,
            is_default: false,
        })
        .await
        .unwrap()
        .project_id;

    let agent_a = db
        .create_agent(DEFAULT_ORG_ID, project_agent_input(proj_a, "alpha"))
        .await
        .unwrap();
    let agent_b = db
        .create_agent(DEFAULT_ORG_ID, project_agent_input(proj_b, "beta"))
        .await
        .unwrap();

    // List is scoped to the active project.
    let (a_only, _) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some(proj_a),
            None,
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(a_only.len(), 1);
    assert_eq!(a_only[0].id, agent_a.id);

    let (b_only, _) = db
        .list_agents(
            DEFAULT_ORG_ID,
            Some(proj_b),
            None,
            false,
            default_pagination(),
        )
        .await
        .unwrap();
    assert_eq!(b_only.len(), 1);
    assert_eq!(b_only[0].id, agent_b.id);

    // None scope is org-wide (internal/worker access).
    let (all, _) = db
        .list_agents(DEFAULT_ORG_ID, None, None, false, default_pagination())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Resolving B's id from project A must miss; org-wide must hit.
    assert!(
        db.get_agent_by_public_id(DEFAULT_ORG_ID, Some(proj_a), &agent_b.public_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_agent_by_public_id(DEFAULT_ORG_ID, None, &agent_b.public_id)
            .await
            .unwrap()
            .is_some()
    );
}

/// Creates a second project in the default org.
async fn second_project(db: &InMemoryDatabase) -> i64 {
    db.create_project(CreateProjectRow {
        public_id: "proj_000000000000000000000000000000cc".to_string(),
        org_id: DEFAULT_ORG_ID,
        name: "Project C".to_string(),
        description: None,
        is_default: false,
    })
    .await
    .unwrap()
    .project_id
}

/// Sessions take their project from what they run (migration 145's
/// `sessions_assign_project`): the agent's, else the parent session's, else
/// the org default. Listing filters by it.
#[tokio::test]
async fn test_sessions_follow_agent_project() {
    let db = InMemoryDatabase::new();
    let proj_c = second_project(&db).await;
    let agent = db
        .create_agent(DEFAULT_ORG_ID, project_agent_input(proj_c, "gamma"))
        .await
        .unwrap();

    let agent_session = db
        .create_session(test_session_input(Some(agent.id)))
        .await
        .unwrap();
    assert_eq!(agent_session.project_id, proj_c);

    let mut child_input = test_session_input(None);
    child_input.parent_session_id = Some(agent_session.id);
    let child = db.create_session(child_input).await.unwrap();
    assert_eq!(
        child.project_id, proj_c,
        "subagent inherits parent's project"
    );

    let agentless = db.create_session(test_session_input(None)).await.unwrap();
    assert_eq!(agentless.project_id, DEFAULT_PROJECT_ID);

    let list = |project_id: Option<i64>| {
        let db = &db;
        async move {
            db.list_sessions(
                DEFAULT_ORG_ID,
                &SessionListFilters {
                    project_id,
                    ..Default::default()
                },
                default_pagination(),
            )
            .await
            .unwrap()
            .0
        }
    };
    assert_eq!(list(Some(proj_c)).await.len(), 2);
    assert_eq!(list(Some(DEFAULT_PROJECT_ID)).await.len(), 1);
    assert_eq!(list(None).await.len(), 3, "None is org-wide");
}

/// An explicit project wins over derivation, but only a project of the
/// session's own org (the Postgres composite foreign key).
#[tokio::test]
async fn test_explicit_session_project_stays_in_org() {
    let db = InMemoryDatabase::new();
    let proj_c = second_project(&db).await;
    let mut input = test_session_input(None);
    input.project_id = Some(proj_c);
    let session = db.create_session(input).await.unwrap();
    assert_eq!(session.project_id, proj_c);

    let other_org_project = db
        .create_project(CreateProjectRow {
            public_id: "proj_000000000000000000000000000000dd".to_string(),
            org_id: DEFAULT_ORG_ID + 1,
            name: "Elsewhere".to_string(),
            description: None,
            is_default: true,
        })
        .await
        .unwrap()
        .project_id;
    let mut input = test_session_input(None);
    input.project_id = Some(other_org_project);
    assert!(
        db.create_session(input).await.is_err(),
        "another org's project is rejected"
    );
}

/// Agent names are unique per project, so the same name coexists across
/// projects and an upsert by name stays inside its project.
#[tokio::test]
async fn test_agent_names_unique_per_project() {
    let db = InMemoryDatabase::new();
    let proj_c = second_project(&db).await;

    let (in_default, created) = db
        .upsert_agent_by_name(
            DEFAULT_ORG_ID,
            project_agent_input(DEFAULT_PROJECT_ID, "shared"),
        )
        .await
        .unwrap();
    assert!(created);
    let (in_c, created) = db
        .upsert_agent_by_name(DEFAULT_ORG_ID, project_agent_input(proj_c, "shared"))
        .await
        .unwrap();
    assert!(created, "same name in another project is a new agent");
    assert_ne!(in_default.id, in_c.id);

    let (again, created) = db
        .upsert_agent_by_name(DEFAULT_ORG_ID, project_agent_input(proj_c, "shared"))
        .await
        .unwrap();
    assert!(!created, "same name in the same project updates");
    assert_eq!(again.id, in_c.id);
}
