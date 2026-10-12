//! PostgreSQL contract for the orphaned managed sandbox scan: which provider
//! accounts it lists with, which listed sandboxes count as orphans, and the
//! stop-then-delete sighting lifecycle.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_capabilities::sandbox_state::SandboxStateStore;
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::session_sandbox::{
    OwnedSandbox, SandboxAdminError, SessionSandboxConfig, SessionSandboxContext,
    SessionSandboxCredentialSource, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::{DEFAULT_ORG_ID, org_public_id_from_internal};
use everruns_server::background::sandbox_orphan_scan::{get_sighting, run_scan, scan_labels};
use everruns_server::domains::sandbox_templates::record::{
    ResolvedSandboxSpec, SandboxBootstrap, SandboxContainmentSpec, SandboxDurability,
    SandboxLifecycle, SandboxNetworkPolicy, SandboxTargetSpec,
};
use everruns_server::storage::{
    CreateOrganizationConnectionRow, Database, PgSandboxCheckpointStore, StorageBackend,
};

use crate::repository_conformance_test::{create_test_principal, session_input};
use crate::test_harness::get_database_url;

const TOKEN: &str = "orphan-scan-token";

/// Returns the token for exactly one connection, so accounts other tests
/// created in the shared database are skipped.
struct OneAccount(Uuid);

#[async_trait]
impl UserConnectionResolver for OneAccount {
    async fn get_connection_token(&self, _: SessionId, _: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }

    async fn get_connection_token_for_connection(
        &self,
        connection_id: Uuid,
        _: Uuid,
        _: &str,
    ) -> CoreResult<Option<String>> {
        Ok((connection_id == self.0).then(|| TOKEN.to_string()))
    }
}

/// A provider whose account holds a fixed set of sandboxes; records actions.
#[derive(Default)]
struct FakeProvider {
    sandboxes: Vec<OwnedSandbox>,
    stopped: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
}

fn unsupported<T>() -> Result<T, ToolExecutionResult> {
    Err(ToolExecutionResult::tool_error("not used by the scan"))
}

/// Shared handle, so the test keeps reading what the scan recorded.
struct SharedFake(Arc<FakeProvider>);

#[async_trait]
impl SessionSandboxProvider for SharedFake {
    fn id(&self) -> &str {
        "daytona"
    }

    async fn list_owned(
        &self,
        token: &str,
        _: &BTreeMap<String, String>,
    ) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
        assert_eq!(token, TOKEN);
        Ok(self.0.sandboxes.clone())
    }

    async fn stop_by_id(&self, _: &str, id: &str) -> Result<(), SandboxAdminError> {
        self.0.stopped.lock().unwrap().push(id.to_string());
        Ok(())
    }

    async fn delete_by_id(&self, _: &str, id: &str) -> Result<(), SandboxAdminError> {
        self.0.deleted.lock().unwrap().push(id.to_string());
        Ok(())
    }

    async fn create(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        unsupported()
    }
    async fn resume(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        unsupported()
    }
    async fn pause(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        unsupported()
    }
    async fn delete(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        unsupported()
    }
    async fn exec(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
        _: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        unsupported()
    }
    async fn read_file(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
        _: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        unsupported()
    }
    async fn write_file(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxInstance,
        _: &str,
        _: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        unsupported()
    }
    async fn status(
        &self,
        _: &dyn SessionSandboxContext,
        _: &SessionSandboxConfig,
        _: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        unsupported()
    }
}

fn listed(
    external_id: &str,
    labels: BTreeMap<String, String>,
    created_at: DateTime<Utc>,
) -> OwnedSandbox {
    OwnedSandbox {
        external_id: external_id.to_string(),
        labels,
        state: "started".to_string(),
        active: true,
        created_at: Some(created_at),
    }
}

fn spec(connection_id: Uuid, virtual_user_id: Uuid) -> ResolvedSandboxSpec {
    let mut target = SandboxTargetSpec::managed("daytona");
    target.credential.source = SessionSandboxCredentialSource::Organization;
    target.credential.connection_id = Some(connection_id);
    target.credential.virtual_user_id = Some(virtual_user_id);
    ResolvedSandboxSpec {
        template_revision_id: None,
        target,
        containment: SandboxContainmentSpec {
            network: SandboxNetworkPolicy::Allow,
            ..SandboxContainmentSpec::isolated()
        },
        durability: SandboxDurability::Checkpointed,
        lifecycle: SandboxLifecycle::default(),
        bootstrap: SandboxBootstrap::default(),
    }
}

fn running(external_id: &str) -> SessionSandboxState {
    let now = Utc::now().to_rfc3339();
    SessionSandboxState {
        sandbox: None,
        provider: "daytona".to_string(),
        status: SessionSandboxStatus::Running,
        instance: SessionSandboxInstance {
            external_id: external_id.to_string(),
            display_name: None,
            workspace_path: None,
            provider_state: json!({}),
            metadata: json!({}),
        },
        init_completed_at: None,
        last_init_error: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

async fn insert_active_lease(pool: &PgPool, external_id: &str) {
    sqlx::query(
        r#"INSERT INTO leased_resources
               (id, public_id, org_id, provider, resource_type, external_id, status,
                lease_duration_seconds, last_touched_at, lease_expires_at)
           VALUES ($1, $2, $3, 'daytona', 'sandbox', $4, 'active', 3600, now(),
                   now() + interval '1 hour')"#,
    )
    .bind(Uuid::new_v4())
    .bind(format!("lease_{}", Uuid::new_v4().simple()))
    .bind(DEFAULT_ORG_ID)
    .bind(external_id)
    .execute(pool)
    .await
    .expect("insert lease");
}

#[tokio::test]
async fn orphans_are_stopped_then_deleted_and_tracked_sandboxes_are_kept() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
    let owner = create_test_principal(&backend, "orphan-scan").await;
    let session = backend
        .create_session(session_input(owner, "orphan-scan"))
        .await
        .expect("create session");
    let connection = backend
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: DEFAULT_ORG_ID,
            name: format!("Daytona orphan scan {}", Uuid::new_v4()),
            provider: "daytona".to_string(),
            access_token_encrypted: vec![1, 2, 3],
            provider_username: None,
            provider_metadata: None,
        })
        .await
        .expect("create account");
    let virtual_user_id = connection.virtual_user_id.uuid();
    backend
        .pin_primary_sandbox(session.id, "build", &spec(connection.id, virtual_user_id))
        .await
        .expect("pin");

    // The account the pinned Sandbox used is the one the scan lists with.
    let accounts = backend
        .list_sandbox_orphan_scan_accounts(&["daytona".to_string()])
        .await
        .expect("accounts");
    assert!(
        accounts.iter().any(|a| a.connection_id == connection.id
            && a.virtual_user_id == virtual_user_id
            && a.org_id == DEFAULT_ORG_ID),
        "{accounts:?}"
    );
    assert!(
        backend
            .list_sandbox_orphan_scan_accounts(&["e2b".to_string()])
            .await
            .expect("accounts")
            .iter()
            .all(|a| a.connection_id != connection.id)
    );

    let suffix = Uuid::new_v4().simple().to_string();
    let tracked = format!("tracked-{suffix}");
    let orphan = format!("orphan-{suffix}");
    let leased = format!("leased-{suffix}");
    let young = format!("young-{suffix}");
    let tool = format!("tool-{suffix}");
    let foreign = format!("foreign-{suffix}");

    PgSandboxCheckpointStore::new(pool.clone())
        .save_state(session.id, &running(&tracked), None)
        .await
        .expect("track instance");
    insert_active_lease(&pool, &leased).await;

    let now = Utc::now();
    let old = now - Duration::hours(2);
    let org = org_public_id_from_internal(DEFAULT_ORG_ID);
    let managed = scan_labels(&org);
    let mut tool_labels = managed.clone();
    tool_labels.remove("everruns.managed");
    let fake = Arc::new(FakeProvider {
        sandboxes: vec![
            listed(&tracked, managed.clone(), old),
            listed(&orphan, managed.clone(), old),
            listed(&leased, managed.clone(), old),
            listed(&young, managed.clone(), now - Duration::minutes(5)),
            listed(&tool, tool_labels, old),
            listed(&foreign, scan_labels("org_other"), old),
        ],
        ..Default::default()
    });
    let lookup_fake = fake.clone();
    let lookup = move |id: &str| -> Option<Box<dyn SessionSandboxProvider>> {
        (id == "daytona")
            .then(|| Box::new(SharedFake(lookup_fake.clone())) as Box<dyn SessionSandboxProvider>)
    };
    let resolver = OneAccount(connection.id);

    // First sighting: only the untracked, unleased, old managed sandbox stops.
    let report = run_scan(&backend, &resolver, &lookup, now)
        .await
        .expect("scan");
    assert_eq!(report.accounts, 1);
    assert_eq!(
        report.candidates, 4,
        "tool and foreign sandboxes fail the re-check"
    );
    assert_eq!(report.stopped, 1);
    assert_eq!(report.deleted, 0);
    assert_eq!(*fake.stopped.lock().unwrap(), vec![orphan.clone()]);
    let sighting = get_sighting(&pool, "daytona", &orphan)
        .await
        .unwrap()
        .expect("sighting recorded");
    assert!(sighting.stopped_at.is_some());
    assert!(
        get_sighting(&pool, "daytona", &tracked)
            .await
            .unwrap()
            .is_none()
    );

    // A day later: the orphan is deleted; the once-young sandbox is now old
    // enough for its own first sighting.
    let report = run_scan(&backend, &resolver, &lookup, now + Duration::hours(25))
        .await
        .expect("scan");
    assert_eq!((report.stopped, report.deleted), (1, 1));
    assert_eq!(*fake.deleted.lock().unwrap(), vec![orphan.clone()]);
    assert_eq!(
        *fake.stopped.lock().unwrap(),
        vec![orphan.clone(), young.clone()]
    );
    assert!(
        get_sighting(&pool, "daytona", &orphan)
            .await
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some()
    );
    assert!(
        fake.deleted
            .lock()
            .unwrap()
            .iter()
            .all(|id| id != &tracked && id != &leased)
    );
}
