// Integration tests for the plugins subsystem.
//
// Tests the full lifecycle:
//   1. Register a local_path marketplace
//   2. Sync to load marketplace.json catalog
//   3. List catalog (microsoft-docs entry present)
//   4. Install plugin → compiled fields present
//   5. GET /v1/capabilities contains plugin:microsoft-docs
//   6. Disable / re-enable
//   7. Update (re-compile)
//   8. Uninstall
//
// These tests use in-memory storage and the local testdata/plugins fixture.
// They require `DEV_MODE=true` (or equivalent deployment grade) so `local_path`
// is accepted. We set `DEPLOYMENT_GRADE=dev` to gate the source-type check.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

/// Absolute path to the testdata plugin fixtures.
fn testdata_plugins_path() -> String {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| "/home/user/everruns/crates/server".to_string());
    // Walk up to the workspace root and into testdata/plugins.
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .unwrap_or_else(|| std::path::Path::new("/home/user/everruns"));
    workspace_root
        .join("testdata")
        .join("plugins")
        .to_string_lossy()
        .to_string()
}

/// Create a TestServer with DEV_MODE so local_path is accepted.
async fn dev_server() -> TestServer {
    // Set DEPLOYMENT_GRADE=dev so validate_source_type accepts local_path.
    unsafe { std::env::set_var("DEPLOYMENT_GRADE", "dev") };
    TestServer::in_memory().await
}

// ============================================================
// Marketplace CRUD
// ============================================================

#[tokio::test]
async fn test_create_marketplace() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let resp = server
        .post(
            "/v1/plugin_marketplaces",
            json!({
                "name": "test-fixtures",
                "source_type": "local_path",
                "source": path
            }),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED, "{}", resp.text());
    let body = resp.json_value();
    assert_eq!(body["name"], "test-fixtures");
    assert_eq!(body["source_type"], "local_path");
    assert_eq!(body["status"], "active");
    assert!(body["id"].as_str().unwrap().starts_with("plgmkt_"));
}

#[tokio::test]
async fn test_list_marketplaces() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "list-test", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let resp = server.get("/v1/plugin_marketplaces").await.assert_success();
    let data = resp.json_value();
    let marketplaces = data["data"].as_array().unwrap();
    assert!(!marketplaces.is_empty());
    assert!(
        marketplaces
            .iter()
            .any(|m| m["name"].as_str() == Some("list-test"))
    );
}

#[tokio::test]
async fn test_get_marketplace() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let create_resp = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "get-test", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let marketplace_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let get_resp = server
        .get(&format!("/v1/plugin_marketplaces/{marketplace_id}"))
        .await
        .assert_success();

    assert_eq!(get_resp.json_value()["name"], "get-test");
}

#[tokio::test]
async fn test_update_marketplace_status() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "update-status", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let updated = server
        .patch(
            &format!("/v1/plugin_marketplaces/{id}"),
            json!({ "status": "disabled" }),
        )
        .await
        .assert_success();

    assert_eq!(updated.json_value()["status"], "disabled");
}

#[tokio::test]
async fn test_delete_marketplace() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "delete-me", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let del_resp = server
        .delete(&format!("/v1/plugin_marketplaces/{id}"))
        .await;
    assert_eq!(
        del_resp.status(),
        StatusCode::NO_CONTENT,
        "{}",
        del_resp.text()
    );

    // Now it should be gone.
    let get_resp = server.get(&format!("/v1/plugin_marketplaces/{id}")).await;
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}

// ============================================================
// Sync + catalog
// ============================================================

#[tokio::test]
async fn test_sync_and_catalog() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    // Create marketplace.
    let id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "sync-catalog-test", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Sync.
    let sync_resp = server
        .post(&format!("/v1/plugin_marketplaces/{id}/sync"), json!({}))
        .await
        .assert_success();

    let sync_body = sync_resp.json_value();
    assert!(
        sync_body["last_synced_at"].is_string(),
        "last_synced_at should be set after sync"
    );

    // List catalog entries.
    let catalog_resp = server
        .get(&format!("/v1/plugin_marketplaces/{id}/plugins"))
        .await
        .assert_success();

    let catalog = catalog_resp.json_value();
    let entries = catalog["data"].as_array().unwrap();
    assert!(
        !entries.is_empty(),
        "catalog should have at least one entry"
    );

    let names: Vec<&str> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(
        names.contains(&"microsoft-docs"),
        "microsoft-docs should be in catalog; got: {:?}",
        names
    );

    // Installed flag should be false before installation.
    let ms_docs = entries
        .iter()
        .find(|e| e["name"].as_str() == Some("microsoft-docs"))
        .unwrap();
    assert_eq!(ms_docs["installed"], false);
}

// ============================================================
// Install + capability integration
// ============================================================

#[tokio::test]
async fn test_install_plugin_and_capability_listing() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    // Register + sync marketplace.
    let marketplace_id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "install-test-mkt", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    server
        .post(
            &format!("/v1/plugin_marketplaces/{marketplace_id}/sync"),
            json!({}),
        )
        .await
        .assert_success();

    // Install the plugin.
    let install_resp = server
        .post(
            "/v1/plugins",
            json!({
                "marketplace_id": marketplace_id,
                "plugin_name": "microsoft-docs"
            }),
        )
        .await;

    assert_eq!(
        install_resp.status(),
        StatusCode::CREATED,
        "install failed: {}",
        install_resp.text()
    );

    let plugin = install_resp.json_value();
    assert_eq!(plugin["name"], "microsoft-docs");
    assert_eq!(plugin["status"], "active");
    assert!(plugin["id"].as_str().unwrap().starts_with("plugin_"));
    assert_eq!(plugin["capability_ref"], "plugin:microsoft-docs");
    assert_eq!(plugin["marketplace"], "install-test-mkt");

    // GET /v1/capabilities should now include plugin:microsoft-docs.
    let caps_resp = server.get("/v1/capabilities").await.assert_success();
    let caps = caps_resp.json_value();
    let cap_ids: Vec<&str> = caps["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(
        cap_ids.contains(&"plugin:microsoft-docs"),
        "capabilities should include plugin:microsoft-docs; got: {:?}",
        cap_ids
    );

    // Catalog installed flag should now be true.
    let catalog_resp = server
        .get(&format!("/v1/plugin_marketplaces/{marketplace_id}/plugins"))
        .await
        .assert_success();
    let ms_docs = catalog_resp.json_value()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"].as_str() == Some("microsoft-docs"))
        .unwrap()
        .clone();
    assert_eq!(ms_docs["installed"], true);
}

// ============================================================
// OAuth MCP plugin: anchor + provider wiring (specs/plugins.md)
// ============================================================

/// Collect the set of `mcp_oauth_*` provider ids from the connections API.
async fn mcp_oauth_providers(server: &TestServer) -> std::collections::BTreeSet<String> {
    server
        .get("/v1/user/connections/providers")
        .await
        .assert_success()
        .json_value()
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["connection_type"].as_str() == Some("oauth"))
        .filter_map(|p| p["provider_id"].as_str())
        .filter(|id| id.starts_with("mcp_oauth_"))
        .map(|s| s.to_string())
        .collect()
}

/// Installing a plugin whose `.mcp.json` marks a server `auth: oauth` must
/// create a disabled anchor `mcp_servers` row that surfaces a new
/// `mcp_oauth_*` provider in the connections API, and remove it on uninstall.
#[tokio::test]
async fn test_install_oauth_plugin_creates_connection_provider() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let marketplace_id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "oauth-mkt", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    server
        .post(
            &format!("/v1/plugin_marketplaces/{marketplace_id}/sync"),
            json!({}),
        )
        .await
        .assert_success();

    let before = mcp_oauth_providers(&server).await;

    let install = server
        .post(
            "/v1/plugins",
            json!({ "marketplace_id": marketplace_id, "plugin_name": "oauth-mail" }),
        )
        .await;
    assert_eq!(
        install.status(),
        StatusCode::CREATED,
        "install failed: {}",
        install.text()
    );
    let plugin_id = install.json_value()["id"].as_str().unwrap().to_string();

    // Exactly one new OAuth provider (the anchor) must appear.
    let after = mcp_oauth_providers(&server).await;
    let new_providers: Vec<&String> = after.difference(&before).collect();
    assert_eq!(
        new_providers.len(),
        1,
        "expected one new mcp_oauth provider; before={before:?} after={after:?}"
    );
    // The anchor's runtime MCP capability id is mcp:{uuid} for the same uuid.
    let anchor_uuid = new_providers[0]
        .strip_prefix("mcp_oauth_")
        .unwrap()
        .to_string();
    let anchor_cap_id = format!("mcp:{anchor_uuid}");

    // The disabled anchor must NOT surface as a runtime MCP capability, and
    // the plugin capability itself must be present.
    let caps = server
        .get("/v1/capabilities")
        .await
        .assert_success()
        .json_value();
    let cap_ids: Vec<&str> = caps["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(
        !cap_ids.contains(&anchor_cap_id.as_str()),
        "disabled anchor {anchor_cap_id} must not be a runtime MCP capability; got {cap_ids:?}"
    );
    assert!(
        cap_ids.contains(&"plugin:oauth-mail"),
        "plugin:oauth-mail capability should be listed; got {cap_ids:?}"
    );

    // Uninstall removes the anchor → provider set returns to baseline.
    server
        .delete(&format!("/v1/plugins/{plugin_id}"))
        .await
        .assert_success();
    let after_uninstall = mcp_oauth_providers(&server).await;
    assert_eq!(
        after_uninstall, before,
        "OAuth provider must be gone after uninstall"
    );
}

// ============================================================
// Plugin lifecycle: disable / re-enable / uninstall
// ============================================================

#[tokio::test]
async fn test_plugin_disable_and_enable() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let marketplace_id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "lifecycle-mkt", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    server
        .post(
            &format!("/v1/plugin_marketplaces/{marketplace_id}/sync"),
            json!({}),
        )
        .await
        .assert_success();

    let plugin_id = server
        .post(
            "/v1/plugins",
            json!({ "marketplace_id": marketplace_id, "plugin_name": "microsoft-docs" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Disable.
    let disabled = server
        .patch(
            &format!("/v1/plugins/{plugin_id}"),
            json!({ "status": "disabled" }),
        )
        .await
        .assert_success();
    assert_eq!(disabled.json_value()["status"], "disabled");

    // Re-enable.
    let enabled = server
        .patch(
            &format!("/v1/plugins/{plugin_id}"),
            json!({ "status": "active" }),
        )
        .await
        .assert_success();
    assert_eq!(enabled.json_value()["status"], "active");
}

#[tokio::test]
async fn test_uninstall_plugin() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let marketplace_id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "uninstall-mkt", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    server
        .post(
            &format!("/v1/plugin_marketplaces/{marketplace_id}/sync"),
            json!({}),
        )
        .await
        .assert_success();

    let plugin_id = server
        .post(
            "/v1/plugins",
            json!({ "marketplace_id": marketplace_id, "plugin_name": "microsoft-docs" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let del = server.delete(&format!("/v1/plugins/{plugin_id}")).await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT, "{}", del.text());

    // Should now be gone.
    let get = server.get(&format!("/v1/plugins/{plugin_id}")).await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

// ============================================================
// Plugin update (re-compile)
// ============================================================

#[tokio::test]
async fn test_update_plugin() {
    let server = dev_server().await;
    let path = testdata_plugins_path();

    let marketplace_id = server
        .post(
            "/v1/plugin_marketplaces",
            json!({ "name": "update-plugin-mkt", "source_type": "local_path", "source": path }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    server
        .post(
            &format!("/v1/plugin_marketplaces/{marketplace_id}/sync"),
            json!({}),
        )
        .await
        .assert_success();

    let plugin_id = server
        .post(
            "/v1/plugins",
            json!({ "marketplace_id": marketplace_id, "plugin_name": "microsoft-docs" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Update (re-compile from current catalog source).
    let updated = server
        .post(&format!("/v1/plugins/{plugin_id}/update"), json!({}))
        .await;

    assert_eq!(
        updated.status(),
        StatusCode::OK,
        "update failed: {}",
        updated.text()
    );
    assert_eq!(updated.json_value()["name"], "microsoft-docs");
}

// ============================================================
// Install-time MCP server validation (TM-PLUGIN-003)
// ============================================================

/// A plugin whose .mcp.json points at a private address must be rejected at
/// install time with a 400, mirroring declarative capability validation.
#[tokio::test]
async fn test_install_rejects_unsafe_mcp_server_url() {
    let server = dev_server().await;

    // Build a temp marketplace with one plugin declaring a localhost MCP URL.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin/marketplace.json"),
        serde_json::to_vec(&json!({
            "name": "unsafe-fixtures",
            "owner": {"name": "Test"},
            "plugins": [{"name": "unsafe-mcp", "source": "./unsafe-mcp",
                         "description": "plugin with private MCP url"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let plugin_dir = root.join("unsafe-mcp");
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin/plugin.json"),
        serde_json::to_vec(&json!({
            "name": "unsafe-mcp",
            "description": "plugin with private MCP url",
            "mcpServers": "./.mcp.json"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers": {"local-loop": {"type": "http", "url": "http://127.0.0.1:8080/mcp"}}
        }))
        .unwrap(),
    )
    .unwrap();

    let resp = server
        .post(
            "/v1/plugin_marketplaces",
            json!({
                "name": "unsafe-fixtures",
                "source_type": "local_path",
                "source": root.to_string_lossy()
            }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "{}", resp.text());
    let mkt_id = resp.json_value()["id"].as_str().unwrap().to_string();

    let resp = server
        .post(&format!("/v1/plugin_marketplaces/{mkt_id}/sync"), json!({}))
        .await;
    assert_eq!(resp.status(), StatusCode::OK, "{}", resp.text());

    let resp = server
        .post(
            "/v1/plugins",
            json!({"marketplace_id": mkt_id, "plugin_name": "unsafe-mcp"}),
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "install with private MCP url must be rejected: {}",
        resp.text()
    );
    assert!(
        resp.text().contains("MCP"),
        "error should mention MCP servers: {}",
        resp.text()
    );
}

// ============================================================
// Default marketplace seeding (specs/plugins.md)
// ============================================================

/// The default "everruns" marketplace is seeded at org creation (here: the
/// in-memory dev server's default org seed path). It must appear in the list
/// immediately after the server starts — no manual registration required.
#[tokio::test]
async fn test_default_marketplace_seeded_at_org_creation() {
    let server = dev_server().await;

    let resp = server.get("/v1/plugin_marketplaces").await.assert_success();
    let data = resp.json_value();
    let marketplaces = data["data"].as_array().unwrap();

    let everruns_mkt = marketplaces
        .iter()
        .find(|m| m["name"].as_str() == Some("everruns"));

    assert!(
        everruns_mkt.is_some(),
        "default 'everruns' marketplace must be seeded at org creation; got: {:?}",
        marketplaces
            .iter()
            .map(|m| m["name"].as_str())
            .collect::<Vec<_>>()
    );

    let mkt = everruns_mkt.unwrap();
    assert_eq!(
        mkt["source_type"].as_str(),
        Some("github"),
        "default marketplace must use source_type 'github'"
    );
    assert_eq!(
        mkt["source"]["repo"].as_str(),
        Some("everruns/everruns"),
        "default marketplace must point at everruns/everruns"
    );
    assert_eq!(
        mkt["status"].as_str(),
        Some("active"),
        "default marketplace must start active"
    );
    assert!(
        mkt["id"].as_str().unwrap().starts_with("plgmkt_"),
        "marketplace id must have plgmkt_ prefix"
    );
    // last_synced_at is NULL until the first explicit sync.
    assert!(
        mkt["last_synced_at"].is_null(),
        "default marketplace must be unsynced on creation, got: {}",
        mkt["last_synced_at"]
    );
}

/// Deleting the default "everruns" marketplace is permanent — it must not
/// reappear on subsequent list calls (no lazy re-seeding on read).
#[tokio::test]
async fn test_default_marketplace_deletion_is_permanent() {
    let server = dev_server().await;

    // Locate the seeded default marketplace.
    let list_resp = server.get("/v1/plugin_marketplaces").await.assert_success();
    let data = list_resp.json_value();
    let marketplaces = data["data"].as_array().unwrap();

    let everruns_mkt = marketplaces
        .iter()
        .find(|m| m["name"].as_str() == Some("everruns"))
        .expect("default 'everruns' marketplace must be seeded");

    let mkt_id = everruns_mkt["id"].as_str().unwrap().to_string();

    // Delete it.
    let del = server
        .delete(&format!("/v1/plugin_marketplaces/{mkt_id}"))
        .await;
    assert_eq!(
        del.status(),
        StatusCode::NO_CONTENT,
        "delete must succeed: {}",
        del.text()
    );

    // List again — the marketplace must be gone and not have been re-seeded.
    let list2 = server.get("/v1/plugin_marketplaces").await.assert_success();
    let data2 = list2.json_value();
    let marketplaces2 = data2["data"].as_array().unwrap();

    assert!(
        !marketplaces2
            .iter()
            .any(|m| m["name"].as_str() == Some("everruns")),
        "deleted default marketplace must not reappear (no lazy re-seeding); got: {:?}",
        marketplaces2
            .iter()
            .map(|m| m["name"].as_str())
            .collect::<Vec<_>>()
    );

    // Direct GET must 404.
    let get = server
        .get(&format!("/v1/plugin_marketplaces/{mkt_id}"))
        .await;
    assert_eq!(
        get.status(),
        StatusCode::NOT_FOUND,
        "deleted marketplace must return 404"
    );
}
