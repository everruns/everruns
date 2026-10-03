// EVE-1172: the canonical `/v1/workspaces/{id}/fs/*` surface must apply the
// same `/memory/user` owner check the legacy session-fs alias got under EVE-63.
// Two users in one org: the session owner and a same-org peer. The peer holds
// workspace view/manage (admin role) and must still be unable to enumerate,
// read, or mutate the owner's private memory, while shared paths stay open.

use super::*;
use crate::auth::AuthConfig;
use crate::storage::models::{CreateMemoryRow, CreateSessionRow};
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use serde_json::json;

const SECRET_PATH: &str = "/memory/user/secret-plan.md";
const SECRET_NAME: &str = "secret-plan.md";

struct Fixture {
    state: AppState,
    workspace_id: String,
    owner: Uuid,
}

fn org_for(user_id: Uuid, role: OrgRole) -> ResolvedOrg {
    ResolvedOrg {
        org_id: DEFAULT_ORG_ID,
        public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Test".to_string(),
        user_id: Some(user_id),
        role,
        is_platform_user: false,
        feature_flags: crate::domains::common::all_feature_flags_for_test(),
    }
}

fn session_row(owner: Uuid) -> CreateSessionRow {
    CreateSessionRow {
        playground_user_id: None,
        source: everruns_platform::SessionSource::Api,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        virtual_user_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: Some(owner),
        title: Some("private memory guard".to_string()),
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
    }
}

/// A default 1:1 workspace whose session is owned by `owner`, with the owner's
/// user-scoped Memory live-mounted at `/memory/user` and one shared file.
async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::in_memory());
    let owner = Uuid::new_v4();
    db.create_memory(
        DEFAULT_ORG_ID,
        CreateMemoryRow {
            public_id: everruns_contracts::typed_id::MemoryId::new().to_string(),
            name: "owner-private".to_string(),
            description: None,
            scope: "user".to_string(),
            owner_agent_id: None,
            owner_user_id: Some(owner),
            source_type: "manual".to_string(),
            source_config: json!({}),
            is_readonly: false,
            sync_status: "idle".to_string(),
            owner_principal_id: None,
            resolved_owner_user_id: Some(owner),
        },
    )
    .await
    .expect("create user memory");
    let session = db
        .create_session(session_row(owner))
        .await
        .expect("create session");
    let state = AppState::new(
        db.clone(),
        AuthState::builtin(AuthConfig::default(), db.clone()),
    );
    for (path, content) in [
        (SECRET_PATH, "<html>private</html>"),
        ("/notes.md", "shared"),
    ] {
        state
            .file_service
            .create_file(
                session.workspace_id,
                CreateFileInput {
                    path: path.to_string(),
                    content: Some(content.to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed file");
    }
    // Precondition: the secret really lives in the mounted Memory, not in
    // plain workspace rows, so the test covers the live mount route.
    assert!(
        state
            .file_service
            .list_all(session.workspace_id)
            .await
            .unwrap()
            .iter()
            .any(|f| f.path == SECRET_PATH)
    );
    Fixture {
        state,
        workspace_id: WorkspaceId::from_uuid(session.workspace_id).to_string(),
        owner,
    }
}

fn names(files: &[FileInfo]) -> Vec<String> {
    files.iter().map(|f| f.path.clone()).collect()
}

fn assert_no_private(paths: &[String]) {
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with("/memory/user") || p.contains(SECRET_NAME)),
        "private memory leaked into listing: {paths:?}"
    );
}

fn assert_forbidden_without_leak(err: (StatusCode, String)) {
    assert_eq!(err.0, StatusCode::FORBIDDEN, "unexpected error: {err:?}");
    assert!(
        !err.1.contains(SECRET_NAME),
        "error leaks filename: {err:?}"
    );
}

async fn listing(
    f: &Fixture,
    org: &ResolvedOrg,
    path: Option<&str>,
    recursive: bool,
) -> Vec<String> {
    match path {
        None => {
            let Json(list) = get_root(
                org.clone(),
                State(f.state.clone()),
                Path(f.workspace_id.clone()),
                Query(GetQuery { recursive }),
            )
            .await
            .expect("root listing");
            names(&list.data)
        }
        Some(path) => {
            let response = get_path(
                org.clone(),
                State(f.state.clone()),
                Path((f.workspace_id.clone(), path.to_string())),
                HeaderMap::new(),
                Query(GetQuery { recursive }),
            )
            .await
            .expect("directory listing");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            value["data"]
                .as_array()
                .expect("listing")
                .iter()
                .map(|f| f["path"].as_str().unwrap().to_string())
                .collect()
        }
    }
}

#[tokio::test]
async fn peer_cannot_enumerate_other_users_private_memory() {
    let f = fixture().await;
    let peer = org_for(Uuid::new_v4(), OrgRole::Member);

    for (path, recursive) in [(None, false), (None, true), (Some("workspace"), true)] {
        let paths = listing(&f, &peer, path, recursive).await;
        assert_no_private(&paths);
        if recursive {
            assert!(paths.contains(&"/notes.md".to_string()), "{paths:?}");
        }
    }
}

#[tokio::test]
async fn peer_cannot_read_other_users_private_memory() {
    let f = fixture().await;
    let peer = org_for(Uuid::new_v4(), OrgRole::Member);
    let ws = || f.workspace_id.clone();
    let st = || State(f.state.clone());

    for path in [
        "memory/user",
        "memory/user/secret-plan.md",
        "memory/user/missing.md",
    ] {
        let err = get_path(
            peer.clone(),
            st(),
            Path((ws(), path.to_string())),
            HeaderMap::new(),
            Query(GetQuery { recursive: false }),
        )
        .await
        .expect_err("read must be denied");
        assert_forbidden_without_leak(err);
    }
    let err = stat_file(
        peer.clone(),
        st(),
        Path(ws()),
        Json(StatRequest {
            path: SECRET_PATH.to_string(),
        }),
    )
    .await
    .expect_err("stat must be denied");
    assert_forbidden_without_leak(err);
    let err = download_path(peer.clone(), st(), Path((ws(), SECRET_PATH.to_string())))
        .await
        .expect_err("download must be denied");
    assert_forbidden_without_leak(err);
    let err = preview_path(
        peer.clone(),
        st(),
        Path((ws(), "memory/user/page.html".to_string())),
    )
    .await
    .expect_err("preview must be denied");
    assert_forbidden_without_leak(err);

    // Shared workspace paths stay readable.
    let _ = stat_file(
        peer.clone(),
        st(),
        Path(ws()),
        Json(StatRequest {
            path: "/notes.md".to_string(),
        }),
    )
    .await
    .expect("shared file stays readable");
}

#[tokio::test]
async fn manager_cannot_mutate_other_users_private_memory() {
    let f = fixture().await;
    let manager = org_for(Uuid::new_v4(), OrgRole::Admin);
    let ws = || f.workspace_id.clone();
    let st = || State(f.state.clone());
    let req = |src: &str, dst: &str| {
        serde_json::from_value::<MoveFileRequest>(json!({"src_path": src, "dst_path": dst}))
            .unwrap()
    };
    let copy_req = |src: &str, dst: &str| {
        serde_json::from_value::<CopyFileRequest>(json!({"src_path": src, "dst_path": dst}))
            .unwrap()
    };

    let err = create_path(
        manager.clone(),
        st(),
        Path((ws(), "memory/user/planted.md".to_string())),
        Json(serde_json::from_value(json!({"content": "x"})).unwrap()),
    )
    .await
    .expect_err("create must be denied");
    assert_forbidden_without_leak(err);
    let err = update_path(
        manager.clone(),
        st(),
        Path((ws(), SECRET_PATH.to_string())),
        Json(serde_json::from_value(json!({"content": "x"})).unwrap()),
    )
    .await
    .expect_err("update must be denied");
    assert_forbidden_without_leak(err);
    for path in [SECRET_PATH, "/memory/user", "/memory"] {
        let err = delete_path(
            manager.clone(),
            st(),
            Path((ws(), path.to_string())),
            Query(DeleteQuery { recursive: true }),
        )
        .await
        .expect_err("delete must be denied");
        assert_forbidden_without_leak(err);
    }
    for (src, dst) in [
        (SECRET_PATH, "/stolen.md"),
        ("/notes.md", "/memory/user/planted.md"),
        ("/memory", "/elsewhere"),
    ] {
        let err = move_file(manager.clone(), st(), Path(ws()), Json(req(src, dst)))
            .await
            .expect_err("move must be denied");
        assert_forbidden_without_leak(err);
        let err = copy_file(manager.clone(), st(), Path(ws()), Json(copy_req(src, dst)))
            .await
            .expect_err("copy must be denied");
        assert_forbidden_without_leak(err);
    }

    // The secret is untouched, and shared paths stay writable for the manager.
    let owner = org_for(f.owner, OrgRole::Member);
    let _ = stat_file(
        owner,
        st(),
        Path(ws()),
        Json(StatRequest {
            path: SECRET_PATH.to_string(),
        }),
    )
    .await
    .expect("owner still sees the secret");
    let _ = copy_file(
        manager.clone(),
        st(),
        Path(ws()),
        Json(copy_req("/notes.md", "/notes-copy.md")),
    )
    .await
    .expect("shared copy stays allowed");
}

#[tokio::test]
async fn owner_keeps_full_access_to_own_private_memory() {
    let f = fixture().await;
    let owner = org_for(f.owner, OrgRole::Admin);
    let ws = || f.workspace_id.clone();
    let st = || State(f.state.clone());

    let paths = listing(&f, &owner, Some("memory/user"), false).await;
    assert!(paths.contains(&SECRET_PATH.to_string()), "{paths:?}");
    let paths = listing(&f, &owner, None, true).await;
    assert!(paths.contains(&SECRET_PATH.to_string()), "{paths:?}");
    download_path(owner.clone(), st(), Path((ws(), SECRET_PATH.to_string())))
        .await
        .expect("owner can download");
    let _ = create_path(
        owner.clone(),
        st(),
        Path((ws(), "memory/user/new.md".to_string())),
        Json(serde_json::from_value(json!({"content": "mine"})).unwrap()),
    )
    .await
    .expect("owner can create");
    let _ = delete_path(
        owner,
        st(),
        Path((ws(), "memory/user/new.md".to_string())),
        Query(DeleteQuery { recursive: false }),
    )
    .await
    .expect("owner can delete");
}
