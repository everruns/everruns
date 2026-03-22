#![cfg(feature = "e2b-live-tests")]

use everruns_integrations_e2b::client::E2BClient;
use everruns_integrations_e2b::state::SandboxState;
use everruns_integrations_e2b::{E2B_DEFAULT_TIMEOUT_SECS, E2B_DEFAULT_WORKSPACE_PATH};
use serde_json::json;

fn get_api_key() -> Option<String> {
    std::env::var("E2B_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Skip test if E2B_API_KEY is not set. Returns the key.
macro_rules! require_api_key {
    () => {
        match get_api_key() {
            Some(key) => key,
            None => {
                eprintln!("[skip] E2B_API_KEY not set, skipping live test");
                return;
            }
        }
    };
}

struct SandboxGuard {
    api_key: String,
    sandbox_id: String,
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let client = E2BClient::new(self.api_key.clone());
        let sandbox_id = self.sandbox_id.clone();
        let handle =
            tokio::runtime::Handle::try_current().expect("tokio runtime required for cleanup");
        // Use block_in_place to allow blocking inside an async context (requires
        // multi-thread runtime, which #[tokio::test] uses by default).
        tokio::task::block_in_place(|| {
            handle.block_on(async move {
                let _ = client.delete_sandbox(&sandbox_id).await;
            });
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smoke_live_sandbox_exec_and_files() {
    let api_key = require_api_key!();
    let client = E2BClient::new(api_key.clone());
    let created = client
        .create_sandbox(
            "base",
            E2B_DEFAULT_TIMEOUT_SECS,
            json!({"everruns": "true", "test": "smoke_live_sandbox_exec_and_files"}),
            json!({"HELLO": "world"}),
        )
        .await
        .expect("create sandbox");
    let _guard = SandboxGuard {
        api_key,
        sandbox_id: created.sandbox_id.clone(),
    };

    eprintln!(
        "[debug] create response: domain={:?}, envd_access_token={:?}, traffic_access_token={:?}",
        created.domain, created.envd_access_token, created.traffic_access_token,
    );

    let detail = client
        .get_sandbox(&created.sandbox_id)
        .await
        .expect("get sandbox detail");

    eprintln!(
        "[debug] detail response: domain={:?}, envd_access_token={:?}",
        detail.domain, detail.envd_access_token,
    );

    let state = SandboxState {
        sandbox_id: detail.sandbox_id.clone(),
        sandbox_domain: detail
            .domain
            .clone()
            .or_else(|| created.domain.clone())
            .unwrap_or_else(|| "e2b.app".to_string()),
        envd_version: detail.envd_version.clone(),
        envd_access_token: detail
            .envd_access_token
            .clone()
            .or_else(|| created.envd_access_token.clone()),
        workspace_path: E2B_DEFAULT_WORKSPACE_PATH.to_string(),
        started_at: detail.started_at.clone(),
        timeout_seconds: E2B_DEFAULT_TIMEOUT_SECS,
    };

    client
        .write_file(&state, "/home/user/hello.txt", "hello from everruns\n")
        .await
        .expect("write file");
    let content = client
        .read_file(&state, "/home/user/hello.txt")
        .await
        .expect("read file");
    assert_eq!(content, "hello from everruns\n");

    let result = client
        .exec(
            &state,
            "pwd && cat /home/user/hello.txt && echo $HELLO",
            Some("/home/user"),
            Some(60_000),
        )
        .await
        .expect("exec command");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("/home/user"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("hello from everruns"),
        "stdout: {}",
        result.stdout
    );
    assert!(result.stdout.contains("world"), "stdout: {}", result.stdout);
}
