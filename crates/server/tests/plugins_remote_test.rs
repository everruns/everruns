// Integration tests for remote (github + url) plugin source support.
//
// Tests cover:
//   (a) Tarball extraction with symlink rejection and size-cap enforcement
//   (b) GitHub marketplace sync (canned API + raw responses)
//   (c) URL marketplace sync (canned response)
//   (d) SSRF rejection at create-time for private-address url source
//   (e) End-to-end install from a faked github marketplace

mod test_harness;

use async_trait::async_trait;
use everruns_core::{
    DEFAULT_ORG_ID, EgressError, EgressRequest, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// FakeEgressService: maps URL → (status, body bytes), bypasses DNS/SSRF
// ---------------------------------------------------------------------------

type ResponseMap = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

#[derive(Clone)]
struct FakeEgressService {
    responses: ResponseMap,
}

impl FakeEgressService {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn add(&self, url: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_string(), (status, body.into()));
    }
}

#[async_trait]
impl EgressService for FakeEgressService {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let map = self.responses.lock().unwrap();
        if let Some((status, body)) = map.get(&request.url) {
            Ok(EgressResponse {
                status: *status,
                headers: Default::default(),
                body: body.clone(),
            })
        } else {
            Err(EgressError::Transport(format!(
                "FakeEgressService: no canned response for '{}'",
                request.url
            )))
        }
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let resp = self.send(request).await?;
        use futures::stream;
        Ok(EgressStreamResponse {
            status: resp.status,
            headers: resp.headers,
            body: Box::pin(stream::once(async move { Ok(resp.body) })),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an in-memory .tar.gz containing the given files + symlinks.
fn build_tarball(root_prefix: &str, files: &[(&str, &[u8])], symlinks: &[(&str, &str)]) -> Vec<u8> {
    use flate2::{Compression, write::GzEncoder};
    use tar::{Builder, EntryType, Header};

    let buf = Vec::new();
    let enc = GzEncoder::new(buf, Compression::fast());
    let mut ar = Builder::new(enc);

    for (path, content) in files {
        let full_path = format!("{root_prefix}{path}");
        let mut header = Header::new_gnu();
        header.set_path(&full_path).unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        ar.append(&header, *content).unwrap();
    }

    for (name, target) in symlinks {
        let full_path = format!("{root_prefix}{name}");
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_path(&full_path).unwrap();
        header.set_link_name(target).unwrap();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        ar.append(&header, &b""[..]).unwrap();
    }

    ar.into_inner().unwrap().finish().unwrap()
}

/// Build a minimal Ctx for unit-level command tests.
async fn make_ctx(
    db: Arc<everruns_server::storage::StorageBackend>,
    egress: Arc<dyn EgressService>,
) -> everruns_server::domains::common::Ctx {
    use everruns_core::{Caller, DefaultPermissionResolver};
    use everruns_server::domains::common::Ctx;
    use everruns_server::services::CapabilityService;

    let cap_svc = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller::internal(DEFAULT_ORG_ID),
        db,
        cap_svc,
        None,
        Arc::new(DefaultPermissionResolver),
    )
    .with_egress_service(egress)
}

async fn seeded_db() -> Arc<everruns_server::storage::StorageBackend> {
    use everruns_core::DeploymentGrade;
    use everruns_server::storage::StorageBackend;
    unsafe { std::env::set_var("DEPLOYMENT_GRADE", "dev") };
    let db = Arc::new(StorageBackend::in_memory());
    let grade = DeploymentGrade::from_env();
    everruns_server::seed::seed_all(
        &db,
        grade,
        &everruns_server::seed::SeedAuthContext::default(),
    )
    .await
    .unwrap();
    db
}

// ---------------------------------------------------------------------------
// (a) Tarball extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_github_fetch_extracts_plugin_from_tarball() {
    use everruns_server::domains::plugins::fetcher::{PluginSource, fetch_plugin};

    let tarball = build_tarball(
        "myrepo-abc123/",
        &[
            (
                "plugins/test-plugin/.claude-plugin/plugin.json",
                br#"{"name":"test-plugin","version":"1.2.3"}"#,
            ),
            ("plugins/test-plugin/README.md", b"# Test Plugin"),
        ],
        &[],
    );

    let fake = FakeEgressService::new();
    fake.add(
        "https://codeload.github.com/myorg/myrepo/tar.gz/abc123",
        200,
        tarball,
    );

    let source = PluginSource {
        source_value: "./plugins/test-plugin".to_string(),
        marketplace_source_type: "github".to_string(),
        marketplace_local_path: None,
        marketplace_github_repo: Some("myorg/myrepo".to_string()),
        marketplace_last_synced_sha: Some("abc123".to_string()),
    };

    let result = fetch_plugin(&source, &(fake as Arc<dyn EgressService>))
        .await
        .unwrap();

    assert_eq!(result.resolved_sha.as_deref(), Some("abc123"));
    assert!(
        result
            .file_set
            .files
            .contains_key(".claude-plugin/plugin.json")
    );
    assert!(result.file_set.files.contains_key("README.md"));
    assert_eq!(result.file_set.dir_name, "test-plugin");
}

#[tokio::test]
async fn test_github_fetch_symlinks_are_silently_skipped() {
    use everruns_server::domains::plugins::fetcher::{PluginSource, fetch_plugin};

    let tarball = build_tarball(
        "repo-sha1/",
        &[("plugin/real.md", b"content")],
        &[("plugin/link.md", "/etc/passwd")],
    );

    let fake = FakeEgressService::new();
    fake.add(
        "https://codeload.github.com/org/repo/tar.gz/sha1",
        200,
        tarball,
    );

    let source = PluginSource {
        source_value: "./plugin".to_string(),
        marketplace_source_type: "github".to_string(),
        marketplace_local_path: None,
        marketplace_github_repo: Some("org/repo".to_string()),
        marketplace_last_synced_sha: Some("sha1".to_string()),
    };

    let result = fetch_plugin(&source, &(fake as Arc<dyn EgressService>))
        .await
        .unwrap();

    assert!(result.file_set.files.contains_key("real.md"));
    assert!(
        !result.file_set.files.contains_key("link.md"),
        "symlinks must be silently skipped"
    );
}

#[tokio::test]
async fn test_github_fetch_rejects_oversized_file() {
    use everruns_core::plugins::MAX_PLUGIN_FILE_BYTES;
    use everruns_server::domains::plugins::fetcher::{PluginSource, fetch_plugin};

    let big = vec![b'x'; MAX_PLUGIN_FILE_BYTES + 1];
    let tarball = build_tarball("repo-sha2/", &[("plugin/big.bin", big.as_slice())], &[]);

    let fake = FakeEgressService::new();
    fake.add(
        "https://codeload.github.com/org/repo/tar.gz/sha2",
        200,
        tarball,
    );

    let source = PluginSource {
        source_value: "./plugin".to_string(),
        marketplace_source_type: "github".to_string(),
        marketplace_local_path: None,
        marketplace_github_repo: Some("org/repo".to_string()),
        marketplace_last_synced_sha: Some("sha2".to_string()),
    };

    let err = fetch_plugin(&source, &(fake as Arc<dyn EgressService>))
        .await
        .unwrap_err();
    assert!(
        err.contains("exceeding the"),
        "expected size error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// (b) GitHub marketplace sync
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_github_marketplace_sync() {
    use everruns_server::domains::common::Command;
    use everruns_server::domains::plugins::commands::{
        CreatePluginMarketplaceCmd, SyncPluginMarketplace,
    };
    use everruns_server::domains::plugins::types::CreatePluginMarketplaceRequest;

    let sha = "deadbeef1234567890abcdef";
    let catalog = json!({
        "plugins": [{"name": "my-plugin", "version": "0.1.0", "source": "./my-plugin"}]
    })
    .to_string();

    let fake = FakeEgressService::new();
    fake.add(
        "https://api.github.com/repos/myorg/myrepo/commits/HEAD",
        200,
        json!({"sha": sha}).to_string(),
    );
    fake.add(
        &format!(
            "https://raw.githubusercontent.com/myorg/myrepo/{sha}/.claude-plugin/marketplace.json"
        ),
        200,
        catalog,
    );

    let db = seeded_db().await;
    let ctx = make_ctx(db.clone(), fake.clone() as Arc<dyn EgressService>).await;

    let marketplace = CreatePluginMarketplaceCmd(CreatePluginMarketplaceRequest {
        name: "github-sync-test".to_string(),
        source_type: "github".to_string(),
        source: "myorg/myrepo".to_string(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let synced = SyncPluginMarketplace {
        id: marketplace.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();

    assert_eq!(synced.last_synced_sha.as_deref(), Some(sha));
    assert!(synced.last_synced_at.is_some());
}

// ---------------------------------------------------------------------------
// (c) URL marketplace sync
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_url_marketplace_sync() {
    use everruns_server::domains::common::Command;
    use everruns_server::domains::plugins::commands::{
        CreatePluginMarketplaceCmd, SyncPluginMarketplace,
    };
    use everruns_server::domains::plugins::types::CreatePluginMarketplaceRequest;

    let catalog = json!({
        "plugins": [{"name": "url-plugin", "version": "2.0.0", "source": "./url-plugin"}]
    })
    .to_string();

    let marketplace_url = "https://example.com/plugins/marketplace.json";

    let fake = FakeEgressService::new();
    fake.add(marketplace_url, 200, catalog);

    let db = seeded_db().await;
    let ctx = make_ctx(db.clone(), fake.clone() as Arc<dyn EgressService>).await;

    let marketplace = CreatePluginMarketplaceCmd(CreatePluginMarketplaceRequest {
        name: "url-sync-test".to_string(),
        source_type: "url".to_string(),
        source: marketplace_url.to_string(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let synced = SyncPluginMarketplace {
        id: marketplace.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();

    // URL sync: no SHA (no commit to pin to).
    assert!(
        synced.last_synced_sha.is_none(),
        "url sync must not set a sha"
    );
    assert!(synced.last_synced_at.is_some());
}

#[tokio::test]
async fn test_url_marketplace_sync_rejects_oversized_body() {
    use everruns_server::domains::common::Command;
    use everruns_server::domains::plugins::commands::{
        CreatePluginMarketplaceCmd, SyncPluginMarketplace,
    };
    use everruns_server::domains::plugins::types::CreatePluginMarketplaceRequest;

    let marketplace_url = "https://example.com/plugins/huge-marketplace.json";

    // Body just over the 1 MB streaming cap (TM-PLUGIN-004).
    let fake = FakeEgressService::new();
    fake.add(marketplace_url, 200, vec![b'x'; 1024 * 1024 + 1]);

    let db = seeded_db().await;
    let ctx = make_ctx(db.clone(), fake.clone() as Arc<dyn EgressService>).await;

    let marketplace = CreatePluginMarketplaceCmd(CreatePluginMarketplaceRequest {
        name: "oversized-sync-test".to_string(),
        source_type: "url".to_string(),
        source: marketplace_url.to_string(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let err = SyncPluginMarketplace {
        id: marketplace.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap_err();

    assert!(
        err.to_string().contains("exceeds"),
        "oversized body must be rejected by the streaming cap, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// (d) SSRF rejection at create-time
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_url_marketplace_create_rejects_private_addresses() {
    use everruns_server::domains::common::{Command, CommandErrorKind};
    use everruns_server::domains::plugins::commands::CreatePluginMarketplaceCmd;
    use everruns_server::domains::plugins::types::CreatePluginMarketplaceRequest;

    let db = seeded_db().await;
    // No fake egress needed — SSRF check is static (no network calls at create time).
    let ctx = make_ctx(db.clone(), Arc::new(everruns_core::DisabledEgressService)).await;

    let bad_urls = [
        "http://127.0.0.1/marketplace.json",
        "http://10.0.0.1/marketplace.json",
        "http://192.168.1.1/marketplace.json",
        "http://169.254.169.254/latest/marketplace.json",
        "http://localhost/marketplace.json",
    ];

    for bad_url in &bad_urls {
        let err = CreatePluginMarketplaceCmd(CreatePluginMarketplaceRequest {
            name: "ssrf-test".to_string(),
            source_type: "url".to_string(),
            source: bad_url.to_string(),
        })
        .execute(&ctx)
        .await
        .unwrap_err();

        assert!(
            matches!(err.kind, CommandErrorKind::BadRequest(_)),
            "expected BadRequest for {bad_url}, got: {:?}",
            err.kind
        );
        let msg = err.message();
        assert!(
            msg.contains("not safe") || msg.contains("Blocked") || msg.contains("private"),
            "expected SSRF message for {bad_url}, got: {msg}"
        );
    }
}

// ---------------------------------------------------------------------------
// (e) End-to-end github install
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_github_marketplace_end_to_end_install() {
    use everruns_server::domains::common::Command;
    use everruns_server::domains::plugins::commands::{
        CreatePluginMarketplaceCmd, InstallPluginCmd, SyncPluginMarketplace,
    };
    use everruns_server::domains::plugins::types::{
        CreatePluginMarketplaceRequest, InstallPluginRequest,
    };

    let sha = "cafebabe1234";

    let tarball = build_tarball(
        &format!("myrepo-{sha}/"),
        &[
            (
                "plugins/e2e-plugin/.claude-plugin/plugin.json",
                br#"{"name":"e2e-plugin","version":"0.5.0","description":"E2E test plugin"}"#,
            ),
            ("plugins/e2e-plugin/README.md", b"# E2E Plugin"),
        ],
        &[],
    );

    let catalog = json!({
        "plugins": [
            {"name": "e2e-plugin", "version": "0.5.0", "source": "./plugins/e2e-plugin"}
        ]
    })
    .to_string();

    let fake = FakeEgressService::new();
    fake.add(
        "https://api.github.com/repos/myorg/myrepo/commits/HEAD",
        200,
        json!({"sha": sha}).to_string(),
    );
    fake.add(
        &format!(
            "https://raw.githubusercontent.com/myorg/myrepo/{sha}/.claude-plugin/marketplace.json"
        ),
        200,
        catalog,
    );
    fake.add(
        &format!("https://codeload.github.com/myorg/myrepo/tar.gz/{sha}"),
        200,
        tarball,
    );

    let db = seeded_db().await;
    let ctx = make_ctx(db.clone(), fake.clone() as Arc<dyn EgressService>).await;

    // 1. Create marketplace.
    let marketplace = CreatePluginMarketplaceCmd(CreatePluginMarketplaceRequest {
        name: "e2e-github".to_string(),
        source_type: "github".to_string(),
        source: "myorg/myrepo".to_string(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    // 2. Sync marketplace.
    let synced = SyncPluginMarketplace {
        id: marketplace.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();

    assert_eq!(synced.last_synced_sha.as_deref(), Some(sha));

    // 3. Install plugin.
    let installed = InstallPluginCmd(InstallPluginRequest {
        marketplace_id: marketplace.public_id.to_string(),
        plugin_name: "e2e-plugin".to_string(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    assert_eq!(installed.name, "e2e-plugin");
    assert_eq!(installed.version.as_deref(), Some("0.5.0"));
    assert_eq!(installed.pinned_sha.as_deref(), Some(sha));
    assert_eq!(installed.status, "active");
    assert_eq!(installed.capability_ref, "plugin:e2e-plugin");
    assert!(
        !installed.update_available,
        "freshly installed is up-to-date"
    );
}
