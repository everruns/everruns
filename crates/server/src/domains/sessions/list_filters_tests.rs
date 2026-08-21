//! Sessions list filters and facet counts (EVE-852).
//!
//! These run against the in-memory backend, which implements the same
//! predicate as the Postgres repository. The Postgres side is covered by
//! `crates/server/tests/repository_integration_test.rs`.

use super::{GetSessionFacets, ListSessions, SessionFilterArgs, SessionService};
use crate::domains::common::{Command, Ctx};
use crate::storage::{CreateAgentRow, CreateEventRow, CreateSessionRow, StorageBackend};
use everruns_core::{Caller, DEFAULT_ORG_ID, DefaultPermissionResolver, OrgRole};
use everruns_platform::SessionSource;
use everruns_provider::typed_id::AgentId;
use everruns_provider::typed_id::{HarnessId, PrincipalId, SessionId};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

fn ctx_for(db: Arc<StorageBackend>, user_id: Option<Uuid>) -> Ctx {
    let session_service = Arc::new(SessionService::new(db.clone()));
    let capability_service = Arc::new(crate::services::CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: everruns_core::organization::org_public_id_from_internal(DEFAULT_ORG_ID),
            user_id,
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: user_id.is_none(),
        },
        db,
        capability_service,
        None,
        Arc::new(DefaultPermissionResolver),
    )
    .with_session_service(session_service)
}

struct Seed {
    source: SessionSource,
    title: &'static str,
    owner_user_id: Option<Uuid>,
    agent_id: Option<AgentId>,
    /// Terminal turn to record, e.g. `turn.completed`.
    last_turn: Option<&'static str>,
}

/// `SessionService::list` resolves a fallback harness for rows without one, so
/// every org used by these tests needs its built-in harnesses provisioned.
async fn new_db(orgs: &[i64]) -> Arc<StorageBackend> {
    let db = Arc::new(StorageBackend::in_memory());
    for org_id in orgs {
        crate::org_init::initialize_org_harnesses(&db, *org_id)
            .await
            .expect("initialize built-in harnesses");
    }
    db
}

async fn seed(db: &Arc<StorageBackend>, spec: Seed) -> SessionId {
    let row = db
        .create_session(CreateSessionRow {
            source: spec.source,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: spec.agent_id,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: spec.owner_user_id,
            title: Some(spec.title.to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .expect("create session");

    if let Some(event_type) = spec.last_turn {
        db.create_event(CreateEventRow {
            session_id: row.id,
            event_type: event_type.to_string(),
            data: json!({}),
            context: json!({}),
            metadata: None,
            tags: None,
            ts: chrono::Utc::now(),
        })
        .await
        .expect("record turn event");
    }

    row.id
}

fn args(source: Option<&str>, status: Option<&str>) -> SessionFilterArgs {
    SessionFilterArgs {
        source: source.map(str::to_string),
        status: status.map(str::to_string),
        ..Default::default()
    }
}

async fn titles(ctx: &Ctx, filters: SessionFilterArgs) -> Vec<String> {
    ListSessions {
        filters,
        offset: None,
        limit: Some(100),
    }
    .execute(ctx)
    .await
    .expect("list sessions")
    .data
    .into_iter()
    .map(|s| s.title.unwrap_or_default())
    .collect()
}

fn bucket(buckets: &[crate::api::sessions::SessionFacetCount], value: &str) -> u64 {
    buckets
        .iter()
        .find(|b| b.value == value)
        .map(|b| b.count)
        .unwrap_or(0)
}

async fn seeded_db() -> Arc<StorageBackend> {
    let db = new_db(&[DEFAULT_ORG_ID]).await;
    seed(
        &db,
        Seed {
            source: SessionSource::Chat,
            title: "chat thread",
            owner_user_id: None,
            agent_id: None,
            last_turn: Some("turn.completed"),
        },
    )
    .await;
    seed(
        &db,
        Seed {
            source: SessionSource::Schedule,
            title: "nightly report",
            owner_user_id: None,
            agent_id: None,
            last_turn: Some("turn.failed"),
        },
    )
    .await;
    seed(
        &db,
        Seed {
            source: SessionSource::Schedule,
            title: "hourly sync",
            owner_user_id: None,
            agent_id: None,
            last_turn: None,
        },
    )
    .await;
    db
}

#[tokio::test]
async fn list_filters_by_source() {
    let ctx = ctx_for(seeded_db().await, None);
    let mut found = titles(&ctx, args(Some("schedule"), None)).await;
    found.sort();
    assert_eq!(found, vec!["hourly sync", "nightly report"]);
}

#[tokio::test]
async fn list_accepts_multiple_sources() {
    let ctx = ctx_for(seeded_db().await, None);
    let found = titles(&ctx, args(Some("chat,schedule"), None)).await;
    assert_eq!(found.len(), 3);
}

#[tokio::test]
async fn list_filters_by_derived_activity() {
    let ctx = ctx_for(seeded_db().await, None);
    assert_eq!(
        titles(&ctx, args(None, Some("failed"))).await,
        vec!["nightly report"]
    );
    assert_eq!(
        titles(&ctx, args(None, Some("completed"))).await,
        vec!["chat thread"]
    );
    // No terminal turn yet — idle, not completed.
    assert_eq!(
        titles(&ctx, args(None, Some("idle"))).await,
        vec!["hourly sync"]
    );
}

#[tokio::test]
async fn filters_compose() {
    let ctx = ctx_for(seeded_db().await, None);
    let found = titles(&ctx, args(Some("schedule"), Some("failed"))).await;
    assert_eq!(found, vec!["nightly report"]);
}

#[tokio::test]
async fn unknown_filter_values_are_rejected_rather_than_ignored() {
    let ctx = ctx_for(seeded_db().await, None);
    // Silently dropping an unrecognized facet would show the caller a wider
    // result set than they asked for.
    for filters in [args(Some("telepathy"), None), args(None, Some("sideways"))] {
        let err = ListSessions {
            filters,
            offset: None,
            limit: None,
        }
        .execute(&ctx)
        .await
        .expect_err("unknown filter value must be rejected");
        assert!(format!("{err:?}").contains("Unknown"), "got {err:?}");
    }
}

#[tokio::test]
async fn mine_restricts_to_the_callers_sessions() {
    let db = new_db(&[DEFAULT_ORG_ID]).await;
    let user = Uuid::now_v7();
    seed(
        &db,
        Seed {
            source: SessionSource::Chat,
            title: "my thread",
            owner_user_id: Some(user),
            agent_id: None,
            last_turn: None,
        },
    )
    .await;
    seed(
        &db,
        Seed {
            source: SessionSource::Chat,
            title: "someone else's thread",
            owner_user_id: Some(Uuid::now_v7()),
            agent_id: None,
            last_turn: None,
        },
    )
    .await;

    let ctx = ctx_for(db, Some(user));
    // This is exactly the Chats sidebar query.
    let found = titles(
        &ctx,
        SessionFilterArgs {
            source: Some("chat".to_string()),
            mine: Some(true),
            order: Some("last_activity".to_string()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(found, vec!["my thread"]);
}

#[tokio::test]
async fn facets_count_every_dimension_over_the_applied_filters() {
    let ctx = ctx_for(seeded_db().await, None);
    let facets = GetSessionFacets {
        filters: SessionFilterArgs::default(),
    }
    .execute(&ctx)
    .await
    .expect("facets");

    assert_eq!(facets.total, 3);
    assert_eq!(bucket(&facets.by_source, "chat"), 1);
    assert_eq!(bucket(&facets.by_source, "schedule"), 2);
    assert_eq!(bucket(&facets.by_activity, "failed"), 1);
    assert_eq!(bucket(&facets.by_activity, "completed"), 1);
    assert_eq!(bucket(&facets.by_activity, "idle"), 1);
    assert_eq!(facets.failed_today, 1);
}

#[tokio::test]
async fn agent_facets_return_public_ids_that_can_be_used_as_filters() {
    let db = new_db(&[DEFAULT_ORG_ID]).await;
    let public_id = AgentId::new();
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: public_id.to_string(),
                name: "facet-agent".to_string(),
                display_name: None,
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                harness_id: HarnessId::from_uuid(Uuid::nil()),
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .expect("create agent");
    seed(
        &db,
        Seed {
            source: SessionSource::Api,
            title: "agent session",
            owner_user_id: None,
            agent_id: Some(agent.id),
            last_turn: None,
        },
    )
    .await;

    let ctx = ctx_for(db, None);
    let facets = GetSessionFacets {
        filters: SessionFilterArgs::default(),
    }
    .execute(&ctx)
    .await
    .expect("facets");
    assert_eq!(bucket(&facets.by_agent, &public_id.to_string()), 1);
    assert_eq!(bucket(&facets.by_agent, &agent.id.to_string()), 0);

    let found = titles(
        &ctx,
        SessionFilterArgs {
            agent_id: Some(public_id),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(found, vec!["agent session"]);
}

#[tokio::test]
async fn a_facet_dimension_excludes_its_own_selection() {
    let ctx = ctx_for(seeded_db().await, None);
    let facets = GetSessionFacets {
        filters: args(Some("chat"), None),
    }
    .execute(&ctx)
    .await
    .expect("facets");

    // The page is narrowed...
    assert_eq!(facets.total, 1);
    // ...but the source rail still offers the other sources, or a
    // multi-select rail would be a dead end after the first click.
    assert_eq!(bucket(&facets.by_source, "chat"), 1);
    assert_eq!(bucket(&facets.by_source, "schedule"), 2);
    // Other dimensions do honour the applied source filter.
    assert_eq!(bucket(&facets.by_activity, "completed"), 1);
    assert_eq!(bucket(&facets.by_activity, "failed"), 0);
}

#[tokio::test]
async fn facets_never_count_across_organizations() {
    let db = new_db(&[DEFAULT_ORG_ID, DEFAULT_ORG_ID + 1]).await;
    seed(
        &db,
        Seed {
            source: SessionSource::Chat,
            title: "ours",
            owner_user_id: None,
            agent_id: None,
            last_turn: None,
        },
    )
    .await;

    let other_org = DEFAULT_ORG_ID + 1;
    db.create_session(CreateSessionRow {
        source: SessionSource::Chat,
        org_id: other_org,
        app_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("theirs".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        system_prompt: None,
        initial_files: json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
        workspace_id: None,
    })
    .await
    .expect("create foreign session");

    let facets = GetSessionFacets {
        filters: SessionFilterArgs::default(),
    }
    .execute(&ctx_for(db, None))
    .await
    .expect("facets");

    assert_eq!(facets.total, 1);
    assert_eq!(bucket(&facets.by_source, "chat"), 1);
}

#[tokio::test]
async fn unknown_agent_filter_yields_an_empty_page_and_zero_counts() {
    let db = seeded_db().await;
    let ctx = ctx_for(db, None);
    let filters = || SessionFilterArgs {
        agent_id: Some(AgentId::new()),
        ..Default::default()
    };

    assert!(titles(&ctx, filters()).await.is_empty());
    let facets = GetSessionFacets { filters: filters() }
        .execute(&ctx)
        .await
        .expect("facets");
    assert_eq!(facets.total, 0);
}

#[test]
fn filter_args_deserialize_through_the_generic_command_runner() {
    // The HTTP layer builds `ListSessions` directly, but the CLI/MCP runner
    // deserializes it from JSON — and `#[serde(flatten)]` changes how the
    // lenient scalar deserializers see their input, so pin it.
    let cmd: ListSessions = serde_json::from_value(json!({
        "source": "chat",
        "status": "running,failed",
        "mine": "true",
        "order": "last_activity",
        "created_after": "2026-08-01T00:00:00Z",
        "limit": "50"
    }))
    .expect("deserialize ListSessions");

    assert_eq!(cmd.filters.source.as_deref(), Some("chat"));
    assert_eq!(cmd.filters.status.as_deref(), Some("running,failed"));
    assert_eq!(cmd.filters.mine, Some(true));
    assert_eq!(cmd.filters.order.as_deref(), Some("last_activity"));
    assert_eq!(cmd.limit, Some(50));
}
