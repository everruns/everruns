//! EVE-1099: the worker's file surface resolves through a registry-aware store.
//!
//! Its own file rather than another block in `tests.rs`, which is on the size
//! ratchet's debt list and may not grow, and because this is a different
//! concern from `tests_sqldb_sharing.rs` (the session SQL database store).
//!
//! The worker's nine session-file operations run as `session_files` domain
//! commands (#3797), and `session_files::queries::service` resolves the store
//! from the `Ctx` — falling back to a bare `WorkspaceFileService` when the ctx
//! carries none. That fallback has no virtual-mount registry, so a session's
//! virtual files simply would not exist for the agent while every unit test
//! that builds its own service still passed.

use super::tests::test_worker_service;
use super::*;
use everruns_core::capability_types::VirtualFileTree;

/// A worker service whose file service is registry-aware, plus the registry.
///
/// `test_worker_service` goes through `WorkerServiceImpl::new`, which passes no
/// registry — so the service it builds is exactly the registry-less one this
/// file is about. Constructing the service here rather than adding a helper to
/// `tests.rs` keeps that file, which is on the size ratchet's debt list, from
/// growing.
async fn worker_service_with_virtual_mounts()
-> (WorkerServiceImpl, Arc<crate::domains::session_files::VirtualMountRegistry>) {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::Dev;
    let host_composition = crate::oss_host_composition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .expect("valid test encryption key"),
    ));

    crate::seed::seed_all(&db, grade, &crate::seed::SeedAuthContext::default())
        .await
        .expect("seed test data");

    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);

    let registry = Arc::new(crate::domains::session_files::VirtualMountRegistry::new());

    let service = WorkerServiceImpl::with_virtual_registry(
        event_service,
        db,
        encryption,
        None,
        host_composition,
        Some(registry.clone()),
        None,
    );

    (service, registry)
}

/// The ctx must carry *a* file service, or the commands build their own.
///
/// This is the cheap half of the guard: it catches the wiring being dropped
/// outright. It cannot tell a registry-aware service from a registry-less one —
/// both are `Some` — which is what the next test is for.
#[tokio::test]
async fn the_grpc_command_ctx_carries_a_file_service() {
    let service = test_worker_service().await;
    let ctx = service.domain_ctx_for_caller(everruns_core::Caller::internal(
        everruns_core::DEFAULT_ORG_ID,
    ));

    assert!(
        ctx.session_file_service.is_some(),
        "without this the session_files commands build their own service via \
         session_files::queries::service's fallback"
    );
}

/// The ctx's file service must actually resolve virtual mounts.
///
/// The regression this pins is the one that got through before: the file RPCs
/// the commands replaced always used the registry-aware service, and the first
/// version of the command path did not, so a session's virtual mounts would
/// have silently stopped existing for the agent. Asserting
/// `session_file_service.is_some()` cannot see that — a registry-less
/// `WorkspaceFileService` is `Some` too. Reading a virtual file back through
/// the exact resolver the commands use can.
#[tokio::test]
async fn the_grpc_command_ctx_file_service_resolves_virtual_mounts() {
    let (service, registry) = worker_service_with_virtual_mounts().await;

    // The registry and the file service are both keyed by the *workspace*, which
    // for the default 1:1 session is the session's own id.
    let workspace_key = uuid::Uuid::now_v7();
    let mut tree = VirtualFileTree::new();
    tree.insert_text("/docs/readme.md", "# Hello from a virtual mount");
    registry.register(
        workspace_key,
        "/docs".into(),
        Arc::new(tree),
        "platform_docs".into(),
    );

    let ctx = service.domain_ctx_for_caller(everruns_core::Caller::internal(
        everruns_core::DEFAULT_ORG_ID,
    ));

    // `queries::service` is the resolver every session_files command goes
    // through, so this asserts the composition at the seam that actually broke
    // rather than at the field.
    let file = crate::domains::session_files::queries::service(&ctx)
        .read_file(workspace_key, "/docs/readme.md")
        .await
        .expect("read through the ctx's file service")
        .expect(
            "the virtual mount must be visible through the ctx's service — a None here means \
             the worker's file commands fell back to a registry-less WorkspaceFileService, and \
             the agent would see an empty /docs",
        );

    assert_eq!(
        file.content.as_deref(),
        Some("# Hello from a virtual mount"),
        "the virtual tree's content must survive the command path's store resolution"
    );
    assert!(
        file.is_readonly,
        "a virtual mount is read-only whatever the store says"
    );
}

/// A directory listing must see the mount too.
///
/// `read_file` alone would still pass if the registry were consulted for exact
/// paths but not for listings, and a listing is how an agent discovers the
/// mount at all — an agent that cannot list `/docs` never reads `readme.md`.
#[tokio::test]
async fn the_grpc_command_ctx_file_service_lists_virtual_mount_entries() {
    let (service, registry) = worker_service_with_virtual_mounts().await;

    let workspace_key = uuid::Uuid::now_v7();
    let mut tree = VirtualFileTree::new();
    tree.insert_text("/docs/readme.md", "# Hello");
    tree.insert_text("/docs/guide.md", "Guide content");
    registry.register(
        workspace_key,
        "/docs".into(),
        Arc::new(tree),
        "platform_docs".into(),
    );

    let ctx = service.domain_ctx_for_caller(everruns_core::Caller::internal(
        everruns_core::DEFAULT_ORG_ID,
    ));

    let entries = crate::domains::session_files::queries::service(&ctx)
        .list_directory(workspace_key, "/docs")
        .await
        .expect("list through the ctx's file service");

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"readme.md") && names.contains(&"guide.md"),
        "the ctx's service must list the virtual mount's entries, got {names:?}"
    );
}
