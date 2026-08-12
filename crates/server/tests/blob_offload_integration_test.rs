//! Backend-agnostic integration coverage for the object-storage blob offload
//! paths (knowledge/runtime-resources/object-storage.md).
//!
//! These tests exercise the *offload* code in
//! `storage/repositories/{session_files,skills}.rs` end-to-end against a real
//! PostgreSQL database plus a `BlobStore`. They are written to be **backend
//! agnostic**: the active blob backend is selected from the environment, so the
//! identical assertions run against
//!
//! - object_store's in-memory backend (the default — `STORAGE_BLOB_BACKEND`
//!   unset / `db`), which exercises the production offload code path with no
//!   network dependency, and
//! - a real S3-compatible service container (`STORAGE_BLOB_BACKEND=s3` +
//!   `STORAGE_S3_*`), wired up by the dedicated CI job.
//!
//! Note: the *default* deployment backend (`STORAGE_BLOB_BACKEND=db`) keeps
//! bytes inline and never attaches a blob store, so there is nothing to offload
//! there. To still get coverage of the offload code on every run we attach the
//! in-memory object_store backend here unless `STORAGE_BLOB_BACKEND=s3` asks for
//! the real S3 path. Either way these tests assert the *offloaded* invariants
//! (empty content column, sidecar pointer + hash, byte round-trip), which is
//! exactly the code that had no automated coverage before.
//!
//! Run with: cargo test -p everruns-server --test blob_offload_integration_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)
//! - Optionally STORAGE_BLOB_BACKEND=s3 + STORAGE_S3_* for the real S3 path.

mod test_harness;

use std::sync::{Arc, OnceLock};

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_server::storage::blob_store::{
    BlobStoreConfig, ObjectStoreBlobStore, SharedBlobStore, content_sha256,
};
use everruns_server::storage::{
    CreateAgentRow, CreateImageRow, CreatePrincipalRow, CreateSessionFileRow, CreateSessionRow,
    Database, StorageBackend, UpdateSessionFile,
};
use test_harness::get_database_url;

const TEST_ORG_ID: i64 = 1;

/// Build the blob backend under test from the environment.
///
/// `STORAGE_BLOB_BACKEND=s3` selects the real S3-compatible store via the same
/// `STORAGE_S3_*` config the server reads in production (used by the dedicated
/// S3 CI job against a SeaweedFS container). Anything else falls back to
/// object_store's in-memory backend, which exercises the identical offload code
/// path with no network dependency. The same test body therefore validates both
/// backends.
///
/// The store is built once per test binary and shared: in the in-memory path a
/// fresh store per call would mean the assertion reads (e.g. "object exists
/// after write") observe a *different* store than the one attached to the
/// `Database`, so they would always fail. Tests run serially
/// (`--test-threads=1`) with unique per-test ids, so sharing one store is safe.
fn blob_store_under_test() -> SharedBlobStore {
    static STORE: OnceLock<SharedBlobStore> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let backend = std::env::var("STORAGE_BLOB_BACKEND").unwrap_or_default();
            if backend == "s3" {
                let config = BlobStoreConfig::from_env()
                    .expect("STORAGE_BLOB_BACKEND=s3 requires STORAGE_S3_* configuration");
                let store = ObjectStoreBlobStore::s3_from_config(&config)
                    .expect("failed to build S3 blob store from STORAGE_S3_* configuration");
                Arc::new(store) as SharedBlobStore
            } else {
                Arc::new(ObjectStoreBlobStore::in_memory()) as SharedBlobStore
            }
        })
        .clone()
}

/// Create a PostgreSQL-backed storage backend with the offload blob store
/// attached. This is the wiring that triggers the offload code paths.
async fn create_offload_backend() -> (StorageBackend, PgPool) {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL");
    let db = Database::new(pool.clone()).with_blob_store(Some(blob_store_under_test()));
    (StorageBackend::Postgres(db), pool)
}

async fn create_test_principal(
    backend: &StorageBackend,
    org_id: i64,
) -> everruns_core::PrincipalId {
    backend
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "blob_offload_integration_test" }),
        })
        .await
        .expect("Failed to create test principal")
        .id
}

async fn create_test_session(backend: &StorageBackend) -> everruns_core::SessionId {
    everruns_server::org_init::initialize_org_harnesses(backend, TEST_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let harness_id = everruns_server::org_init::generic_harness_id(backend, TEST_ORG_ID)
        .await
        .expect("generic harness id");
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                // Full UUID, not a truncated prefix: the leading chars of a v7
                // UUID are shared timestamp bits, so a truncated prefix collides
                // for agents created in the same window (unique (org_id, name)).
                name: format!("blob-offload-agent-{}", Uuid::now_v7()),
                display_name: Some("Blob Offload Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                harness_id,
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
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(backend, TEST_ORG_ID).await;
    backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .expect("Failed to create session")
        .id
}

/// The raw `content` column for a workspace file (NULL when offloaded).
async fn raw_file_content(pool: &PgPool, workspace_id: Uuid, path: &str) -> Option<Vec<u8>> {
    let row: (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT content FROM workspace_files WHERE workspace_id = $1 AND path = $2")
            .bind(workspace_id)
            .bind(path)
            .fetch_one(pool)
            .await
            .expect("file row must exist");
    row.0
}

/// The sidecar pointer (blob_key, content_sha256) for a workspace file, if any.
async fn file_sidecar(pool: &PgPool, workspace_id: Uuid, path: &str) -> Option<(String, String)> {
    sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT b.blob_key, b.content_sha256
        FROM workspace_file_blobs b
        JOIN workspace_files f ON f.id = b.file_id
        WHERE f.workspace_id = $1 AND f.path = $2
        "#,
    )
    .bind(workspace_id)
    .bind(path)
    .fetch_optional(pool)
    .await
    .expect("sidecar query")
}

// ============================================================================
// Workspace file offload
// ============================================================================

#[tokio::test]
async fn offloaded_file_empties_content_column_and_records_sidecar() {
    let (backend, pool) = create_offload_backend().await;
    let session_id = create_test_session(&backend).await;
    let workspace_id: Uuid = session_id.into();
    let body = b"Hello, offloaded world!".to_vec();

    let created = backend
        .create_session_file(CreateSessionFileRow {
            session_id,
            path: "/notes/a.txt".to_string(),
            is_directory: false,
            content: Some(body.clone()),
            is_readonly: false,
        })
        .await
        .expect("create offloaded file");

    // The returned row carries the bytes (materialized for the caller).
    assert_eq!(created.content.as_deref(), Some(body.as_slice()));
    assert_eq!(created.size_bytes, body.len() as i64);

    // The DB content column is empty (offloaded), and the sidecar records the
    // pointer + hash.
    assert_eq!(
        raw_file_content(&pool, workspace_id, "/notes/a.txt").await,
        None,
        "offloaded file must have NULL content column"
    );
    let (key, sha) = file_sidecar(&pool, workspace_id, "/notes/a.txt")
        .await
        .expect("sidecar pointer must exist for offloaded file");
    assert!(
        key.contains(&workspace_id.to_string()),
        "key is tenant-scoped"
    );
    assert_eq!(sha, content_sha256(&body), "sidecar records content hash");

    // Round-trip on read fetches the bytes from the blob store.
    let fetched = backend
        .get_session_file(workspace_id, "/notes/a.txt")
        .await
        .expect("get file")
        .expect("file present");
    assert_eq!(fetched.content.as_deref(), Some(body.as_slice()));

    backend.delete_session(TEST_ORG_ID, session_id).await.ok();
}

#[tokio::test]
async fn offloaded_file_delete_removes_object_and_sidecar() {
    let (backend, pool) = create_offload_backend().await;
    let session_id = create_test_session(&backend).await;
    let workspace_id: Uuid = session_id.into();

    backend
        .create_session_file(CreateSessionFileRow {
            session_id,
            path: "/gone.txt".to_string(),
            is_directory: false,
            content: Some(b"delete me".to_vec()),
            is_readonly: false,
        })
        .await
        .expect("create file");

    // Grab the blob store + key so we can prove the object is gone after delete.
    let blob = blob_store_under_test();
    let (key, _) = file_sidecar(&pool, workspace_id, "/gone.txt")
        .await
        .expect("sidecar exists");
    assert!(
        blob.get(&key).await.expect("blob get").is_some(),
        "object exists before delete"
    );

    let deleted = backend
        .delete_session_file(workspace_id, "/gone.txt")
        .await
        .expect("delete file");
    assert!(deleted);

    // Object gone, and the row (hence cascade sidecar) gone.
    assert!(
        blob.get(&key).await.expect("blob get").is_none(),
        "delete must remove the backing object"
    );
    assert!(
        file_sidecar(&pool, workspace_id, "/gone.txt")
            .await
            .is_none(),
        "sidecar pointer must be gone after delete"
    );

    backend.delete_session(TEST_ORG_ID, session_id).await.ok();
}

#[tokio::test]
async fn offloaded_file_update_writes_immutable_blob_revision_and_hash() {
    let (backend, pool) = create_offload_backend().await;
    let session_id = create_test_session(&backend).await;
    let workspace_id: Uuid = session_id.into();

    let original_body = b"v1".to_vec();
    backend
        .create_session_file(CreateSessionFileRow {
            session_id,
            path: "/edit.txt".to_string(),
            is_directory: false,
            content: Some(original_body.clone()),
            is_readonly: false,
        })
        .await
        .expect("create file");

    let (old_key, old_sha) = file_sidecar(&pool, workspace_id, "/edit.txt")
        .await
        .expect("initial sidecar exists");
    assert_eq!(old_sha, content_sha256(&original_body));

    let new_body = b"v2 longer content".to_vec();
    let updated = backend
        .update_session_file(
            workspace_id,
            "/edit.txt",
            UpdateSessionFile {
                content: Some(new_body.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("update file")
        .expect("file present");
    assert_eq!(updated.content.as_deref(), Some(new_body.as_slice()));
    assert_eq!(updated.size_bytes, new_body.len() as i64);

    // Content column still empty; sidecar hash now reflects the new bytes.
    assert_eq!(
        raw_file_content(&pool, workspace_id, "/edit.txt").await,
        None
    );
    let (new_key, sha) = file_sidecar(&pool, workspace_id, "/edit.txt")
        .await
        .expect("sidecar exists");
    assert_eq!(sha, content_sha256(&new_body));
    assert_ne!(
        new_key, old_key,
        "content updates must point at an immutable blob revision"
    );

    let blob = blob_store_under_test();
    assert_eq!(
        blob.get(&old_key).await.expect("old blob get").as_deref(),
        None,
        "unique superseded revisions are reclaimed immediately"
    );
    assert_eq!(
        blob.get(&new_key).await.expect("new blob get").as_deref(),
        Some(new_body.as_slice())
    );

    // Read returns the new bytes from the blob store.
    let fetched = backend
        .get_session_file(workspace_id, "/edit.txt")
        .await
        .expect("get file")
        .expect("file present");
    assert_eq!(fetched.content.as_deref(), Some(new_body.as_slice()));

    backend.delete_session(TEST_ORG_ID, session_id).await.ok();
}

#[tokio::test]
async fn offloaded_file_cas_matches_and_rejects() {
    let (backend, pool) = create_offload_backend().await;
    let session_id = create_test_session(&backend).await;
    let workspace_id: Uuid = session_id.into();

    let original = b"original".to_vec();
    backend
        .create_session_file(CreateSessionFileRow {
            session_id,
            path: "/cas.txt".to_string(),
            is_directory: false,
            content: Some(original.clone()),
            is_readonly: false,
        })
        .await
        .expect("create file");

    // Stale expected content is rejected (no change).
    let rejected = backend
        .update_session_file_if_content_matches(
            workspace_id,
            "/cas.txt",
            b"WRONG".to_vec(),
            UpdateSessionFile {
                content: Some(b"should not apply".to_vec()),
                ..Default::default()
            },
        )
        .await
        .expect("cas call");
    assert!(rejected.is_none(), "CAS must reject a stale expected value");
    // The blob is unchanged.
    let fetched = backend
        .get_session_file(workspace_id, "/cas.txt")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.content.as_deref(), Some(original.as_slice()));

    let (old_cas_key, _) = file_sidecar(&pool, workspace_id, "/cas.txt")
        .await
        .expect("sidecar exists before CAS");

    // Correct expected content applies the write.
    let new_body = b"second revision".to_vec();
    let applied = backend
        .update_session_file_if_content_matches(
            workspace_id,
            "/cas.txt",
            original.clone(),
            UpdateSessionFile {
                content: Some(new_body.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("cas call")
        .expect("CAS must apply when expected matches");
    assert_eq!(applied.content.as_deref(), Some(new_body.as_slice()));
    assert_eq!(
        raw_file_content(&pool, workspace_id, "/cas.txt").await,
        None
    );
    let (new_cas_key, sha) = file_sidecar(&pool, workspace_id, "/cas.txt")
        .await
        .expect("sidecar exists");
    assert_eq!(sha, content_sha256(&new_body));
    assert_ne!(new_cas_key, old_cas_key);
    let blob = blob_store_under_test();
    assert_eq!(
        blob.get(&old_cas_key)
            .await
            .expect("old CAS blob get")
            .as_deref(),
        None,
        "unique CAS superseded revisions are reclaimed immediately"
    );

    backend.delete_session(TEST_ORG_ID, session_id).await.ok();
}

#[tokio::test]
async fn offloaded_file_move_keeps_blob_copy_duplicates_it() {
    let (backend, pool) = create_offload_backend().await;
    let session_id = create_test_session(&backend).await;
    let workspace_id: Uuid = session_id.into();
    let body = b"movable".to_vec();

    backend
        .create_session_file(CreateSessionFileRow {
            session_id,
            path: "/src.txt".to_string(),
            is_directory: false,
            content: Some(body.clone()),
            is_readonly: false,
        })
        .await
        .expect("create file");

    let (src_key, _) = file_sidecar(&pool, workspace_id, "/src.txt")
        .await
        .expect("sidecar exists");

    // Copy duplicates the blob under a new object key (independent file id).
    let copied = backend
        .copy_session_file(workspace_id, "/src.txt", "/copy.txt")
        .await
        .expect("copy")
        .expect("copy returns row");
    assert_eq!(copied.content.as_deref(), Some(body.as_slice()));
    let (copy_key, copy_sha) = file_sidecar(&pool, workspace_id, "/copy.txt")
        .await
        .expect("copy sidecar exists");
    assert_ne!(copy_key, src_key, "copy gets its own object key");
    assert_eq!(copy_sha, content_sha256(&body));

    // Move only rewrites path metadata: the object/key is untouched, and the
    // bytes round-trip at the new path.
    let moved = backend
        .move_session_file(workspace_id, "/src.txt", "/moved.txt")
        .await
        .expect("move")
        .expect("move returns row");
    assert_eq!(moved.content.as_deref(), Some(body.as_slice()));
    let (moved_key, _) = file_sidecar(&pool, workspace_id, "/moved.txt")
        .await
        .expect("moved sidecar exists");
    assert_eq!(moved_key, src_key, "move keeps the same backing object");
    assert!(
        file_sidecar(&pool, workspace_id, "/src.txt")
            .await
            .is_none(),
        "old path no longer resolves"
    );

    backend.delete_session(TEST_ORG_ID, session_id).await.ok();
}

// ============================================================================
// Image offload
// ============================================================================

#[tokio::test]
async fn offloaded_image_empties_data_column_and_round_trips() {
    let (backend, pool) = create_offload_backend().await;
    let data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A]; // PNG-ish header
    let thumb = vec![0x01, 0x02, 0x03];

    let image = backend
        .create_image(
            TEST_ORG_ID,
            CreateImageRow {
                org_id: TEST_ORG_ID,
                filename: "offloaded.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: data.len() as i64,
                data: data.clone(),
                thumbnail_data: Some(thumb.clone()),
                thumbnail_content_type: Some("image/png".to_string()),
                metadata: json!({}),
            },
        )
        .await
        .expect("create image");

    // The DB `data` column is empty (offloaded to the object store).
    let raw: (Vec<u8>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT data, thumbnail_data FROM images WHERE id = $1")
            .bind(image.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("image row");
    assert!(
        raw.0.is_empty(),
        "offloaded image data column must be empty"
    );
    assert!(
        raw.1.is_none(),
        "offloaded image thumbnail column must be empty"
    );

    // Sidecar pointers exist.
    let pointers: (String, Option<String>) =
        sqlx::query_as("SELECT data_key, thumbnail_key FROM image_blobs WHERE image_id = $1")
            .bind(image.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("image_blobs row");
    assert!(pointers.0.contains(&format!("org-{TEST_ORG_ID}")));
    assert!(pointers.1.is_some(), "thumbnail pointer recorded");

    // Read materializes bytes from the blob store.
    let fetched = backend
        .get_image(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("get image")
        .expect("image present");
    assert_eq!(fetched.data, data);
    assert_eq!(fetched.thumbnail_data.as_deref(), Some(thumb.as_slice()));

    // Delete removes the backing objects.
    let blob = blob_store_under_test();
    assert!(blob.get(&pointers.0).await.expect("get").is_some());
    backend
        .delete_image(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("delete image");
    assert!(
        blob.get(&pointers.0).await.expect("get").is_none(),
        "delete must remove the image data object"
    );
}

#[tokio::test]
async fn offloaded_image_delete_is_org_scoped() {
    // Cross-tenant guard: a delete issued for the wrong org must not remove the
    // owning org's objects (the lookup joins images filtered by org_id).
    let (backend, pool) = create_offload_backend().await;

    // A second org to act as the attacker tenant.
    let other_org = backend
        .create_organization(everruns_server::storage::CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: format!("Other Org {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("create other org");

    let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let image = backend
        .create_image(
            TEST_ORG_ID,
            CreateImageRow {
                org_id: TEST_ORG_ID,
                filename: "secret.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: data.len() as i64,
                data: data.clone(),
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .expect("create image");

    let pointers: (String, Option<String>) =
        sqlx::query_as("SELECT data_key, thumbnail_key FROM image_blobs WHERE image_id = $1")
            .bind(image.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("image_blobs row");

    // Attacker org tries to delete the victim org's image id.
    let deleted = backend
        .delete_image(other_org.org_id, image.id.uuid())
        .await
        .expect("delete call");
    assert!(
        !deleted,
        "cross-org delete must not affect another org's image"
    );

    // The object and the row are untouched, and the owning org can still read it.
    let blob = blob_store_under_test();
    assert!(
        blob.get(&pointers.0).await.expect("get").is_some(),
        "victim object must survive a cross-org delete attempt"
    );
    let still_there = backend
        .get_image(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("get image")
        .expect("victim image must still exist");
    assert_eq!(still_there.data, data);

    // Cleanup.
    backend
        .delete_image(TEST_ORG_ID, image.id.uuid())
        .await
        .ok();
    backend.delete_organization(other_org.org_id).await.ok();
}
