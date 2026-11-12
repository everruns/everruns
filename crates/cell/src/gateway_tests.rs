//! Authorization reads must not trust a lagging replica's session view.

use super::*;
use crate::org_token::{OrgTokenClaims, OrgTokenClaimsExt};
use crate::state::{CellState, Left, SessionState};

async fn gateway(state: Arc<Mutex<CellState>>) -> (Arc<Gateway>, tempfile::TempDir) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // Model a follower that has applied Create but not Assign. Proposals
    // are forwarded to the leader, whose state includes committed Assign.
    let app = Router::new()
        .route("/cell/sessions/{session}", get(|Path(session): Path<String>| async move {
            Json(json!({"found": "here", "value": {
                "id": session, "agent": "echo", "last_sequence": 0,
                "inbox": 0, "active": null, "turns_completed": 0,
                "placement": "live", "tenant": null
            }}))
        }))
        .route("/cell/propose", post(|State(state): State<Arc<Mutex<CellState>>>, Json(command): Json<Command>| async move {
            Json(Proposed { response: state.lock().apply(command, &mut Vec::new()), index: 1 })
        }))
        .with_state(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("topology.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&crate::cluster::Topology {
            directory_cell: 0,
            cells: BTreeMap::from([(0, BTreeMap::from([(1, addr)]))]),
        })
        .unwrap(),
    )
    .unwrap();
    (Gateway::new(TopologySource { path }), dir)
}

#[tokio::test]
async fn tenant_lookup_uses_committed_assignment_not_lagging_replica() {
    let session = SessionId::new();
    let mut state = CellState::default();
    state.sessions.insert(
        session,
        SessionState {
            tenant: Some("org-a".into()),
            ..Default::default()
        },
    );
    let (gateway, _dir) = gateway(Arc::new(Mutex::new(state))).await;
    gateway.cache.lock().insert(session, 0);
    let tenant = gateway.session_tenant(session).await.unwrap();
    assert_eq!(tenant.as_deref(), Some("org-a"));
    let mut claims = OrgTokenClaims {
        sub: "user-1".into(),
        org: "org-a".into(),
        dp: "dp-local".into(),
        scope: "sessions:write".into(),
        iat: 0,
        exp: 1,
        jti: "token".into(),
    };
    assert!(claims.may_drive(tenant.as_deref()));
    claims.org = "org-b".into();
    assert!(!claims.may_drive(tenant.as_deref()));
}

#[tokio::test]
async fn tenant_lookup_does_not_cache_before_assignment() {
    let session = SessionId::new();
    let state = Arc::new(Mutex::new(CellState::default()));
    state
        .lock()
        .sessions
        .insert(session, SessionState::default());
    let (gateway, _dir) = gateway(state.clone()).await;
    gateway.cache.lock().insert(session, 0);
    assert_eq!(gateway.session_tenant(session).await.unwrap(), None);
    state.lock().apply(
        Command::Assign {
            session,
            tenant: "org-a".into(),
        },
        &mut Vec::new(),
    );
    assert_eq!(
        gateway.session_tenant(session).await.unwrap().as_deref(),
        Some("org-a")
    );
}

#[test]
fn tenant_lookup_preserves_missing_and_moved_sessions() {
    let session = SessionId::new();
    let mut state = CellState::default();
    assert!(matches!(
        state.apply(Command::Tenant { session }, &mut Vec::new()),
        Response::NotFound
    ));
    state.forwarded.insert(
        session,
        Left {
            cell: 2,
            generation: 1,
        },
    );
    assert!(matches!(
        state.apply(Command::Tenant { session }, &mut Vec::new()),
        Response::Moved { cell: 2 }
    ));
}
