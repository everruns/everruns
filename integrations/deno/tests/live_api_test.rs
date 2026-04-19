//! Live Deno sandbox integration tests.
//!
//! Gated behind:
//! - Feature flag: `deno-live-tests`
//! - Environment variable: `DENO_DEPLOY_TOKEN` (required; missing ⇒ panic)
//! - Environment variable: `DENO_DEPLOY_ORG` (required when the token is a
//!   personal `ddp_...` token; missing in that case ⇒ panic)
//!
//! Missing-credential policy: these tests fail closed. If the feature flag is
//! set but credentials are missing or inconsistent, the tests panic rather
//! than silently passing, so CI live jobs cannot report false-green.
//! See `specs/integrations.md`.
//!
//! Run with:
//!   doppler run -- cargo test -p everruns-integrations-deno \
//!     --features deno-live-tests --test live_api_test -- --test-threads=1 --nocapture

#![cfg(feature = "deno-live-tests")]

use everruns_integrations_deno::client::{CreateSandboxRequest, DenoClient};
use serde_json::Value;

struct SandboxGuard {
    sandbox_id: String,
    region: String,
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let sandbox_id = self.sandbox_id.clone();
        let region = self.region.clone();
        let Some(token) = std::env::var("DENO_DEPLOY_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
        else {
            eprintln!("[cleanup] No DENO_DEPLOY_TOKEN, cannot delete sandbox {sandbox_id}");
            return;
        };
        let org = std::env::var("DENO_DEPLOY_ORG")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("cleanup runtime");
            rt.block_on(async move {
                let client = DenoClient::new(token, org);
                match client.delete_sandbox(&sandbox_id, &region).await {
                    Ok(()) => eprintln!("[cleanup] Deleted sandbox {sandbox_id}"),
                    Err(error) => {
                        eprintln!("[cleanup] Failed to delete sandbox {sandbox_id}: {error}")
                    }
                }
            });
        });
        let _ = handle.join();
    }
}

fn token() -> Option<String> {
    std::env::var("DENO_DEPLOY_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
}

fn org() -> Option<String> {
    std::env::var("DENO_DEPLOY_ORG")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// Require `DENO_DEPLOY_TOKEN` (plus `DENO_DEPLOY_ORG` for personal `ddp_...`
/// tokens) or panic. Live tests fail closed so CI cannot silently pass when
/// credentials are missing. See `specs/integrations.md`.
macro_rules! require_token {
    () => {{
        match token() {
            Some(token) => {
                if token.starts_with("ddp_") && org().is_none() {
                    panic!(
                        "DENO_DEPLOY_TOKEN is personal (ddp_...) but DENO_DEPLOY_ORG is unset — live tests require real credentials (fail-closed policy)"
                    );
                }
                token
            }
            None => panic!(
                "DENO_DEPLOY_TOKEN not set — live tests require real credentials (fail-closed policy)"
            ),
        }
    }};
}

#[tokio::test]
async fn test_live_sandbox_lifecycle() {
    let token = require_token!();
    let client = DenoClient::new(token, org());
    let created = client
        .create_sandbox(CreateSandboxRequest {
            region: None,
            timeout_seconds: Some(10 * 60),
            memory_mb: Some(1_280),
            labels: serde_json::Map::from_iter([(
                String::from("everruns-test"),
                Value::String(String::from("live")),
            )]),
            allow_net: vec![],
        })
        .await
        .expect("create sandbox");
    let _guard = SandboxGuard {
        sandbox_id: created.sandbox_id.clone(),
        region: created.region.clone(),
    };

    let exec = client
        .exec(
            &created.sandbox_id,
            &created.region,
            "echo hello-deno && pwd",
            None,
        )
        .await
        .expect("exec");
    assert_eq!(exec.exit_code, 0);
    assert!(
        exec.stdout.contains("hello-deno"),
        "stdout: {}",
        exec.stdout
    );
    assert!(
        exec.stdout.contains("/home/sandbox"),
        "stdout: {}",
        exec.stdout
    );

    client
        .write_text_file(
            &created.sandbox_id,
            &created.region,
            "/home/sandbox/live.txt",
            "hello from everruns\n",
        )
        .await
        .expect("write file");
    let content = client
        .read_text_file(
            &created.sandbox_id,
            &created.region,
            "/home/sandbox/live.txt",
        )
        .await
        .expect("read file");
    assert_eq!(content, "hello from everruns\n");
}
