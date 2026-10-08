#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live E2B API integration tests.
//!
//! Gated behind:
//! - Feature flag: `e2b-live-tests`
//! - Environment variable: `E2B_API_KEY` (required; missing ⇒ panic)
//!
//! Missing-credential policy: these tests fail closed. If the feature flag is
//! set but the credential is missing, the tests panic rather than silently
//! passing, so CI live jobs cannot report false-green. See `knowledge/integrations/integrations.md`.

#![cfg(feature = "e2b-live-tests")]

use everruns_integrations::e2b::client::E2BClient;
use everruns_integrations::e2b::state::SandboxState;
use everruns_integrations::e2b::{E2B_DEFAULT_TIMEOUT_SECS, E2B_DEFAULT_WORKSPACE_PATH};
use serde_json::json;

fn get_api_key() -> Option<String> {
    std::env::var("E2B_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Require `E2B_API_KEY` or panic. Live tests fail closed so CI cannot
/// silently pass when the credential is missing. See `knowledge/integrations/integrations.md`.
macro_rules! require_api_key {
    () => {
        match get_api_key() {
            Some(key) => key,
            None => panic!(
                "E2B_API_KEY not set — live tests require real credentials (fail-closed policy)"
            ),
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
    // Test binary doesn't go through init_telemetry() or CLI main(),
    // so install the rustls CryptoProvider explicitly (rustls 0.23 requirement).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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

    let detail = client
        .get_sandbox(&created.sandbox_id)
        .await
        .expect("get sandbox detail");
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

/// Desktop computer use (EVE-1133): bring up Xvfb in the `desktop` template,
/// move the pointer, type hostile text into a terminal with no shell in the
/// path, and read a frame back at the display size.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smoke_live_desktop_computer_use() {
    use everruns_contracts::runtime::computer_use::{ComputerAction, ComputerSession, DisplaySize};
    use everruns_integrations::e2b::computer::{
        DESKTOP_DISPLAY, E2B_DESKTOP_TEMPLATE, E2BDesktopSession, png_dimensions,
    };
    use everruns_integrations::e2b::state::build_state;

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let api_key = require_api_key!();
    let client = E2BClient::new(api_key.clone());
    let created = client
        .create_sandbox(
            E2B_DESKTOP_TEMPLATE,
            600,
            json!({"everruns": "true", "test": "smoke_live_desktop_computer_use"}),
            json!({}),
        )
        .await
        .expect("create desktop sandbox");
    let _guard = SandboxGuard {
        api_key: api_key.clone(),
        sandbox_id: created.sandbox_id.clone(),
    };
    let mut detail = client
        .get_sandbox(&created.sandbox_id)
        .await
        .expect("get sandbox detail");
    detail.domain = detail.domain.or(created.domain.clone());
    detail.envd_access_token = detail
        .envd_access_token
        .or(created.envd_access_token.clone());
    let state = build_state(&detail, 600);

    let display = DisplaySize {
        width: 1024,
        height: 768,
    };
    let mut session = E2BDesktopSession::start(E2BClient::new(api_key), state.clone(), display)
        .await
        .expect("start the desktop display");

    let shot = session.screenshot().await.expect("screenshot");
    assert_eq!(shot.media_type, "image/png");

    session
        .perform(&ComputerAction::MouseMove {
            coordinate: [123, 456],
        })
        .await
        .expect("mouse_move");
    let location = client
        .exec_argv(
            &state,
            "xdotool",
            &["getmouselocation".to_string()],
            &[("DISPLAY", DESKTOP_DISPLAY)],
            None,
            Some(30_000),
        )
        .await
        .expect("getmouselocation");
    assert!(
        location.stdout.contains("x:123 y:456"),
        "pointer: {}",
        location.stdout
    );
    assert_eq!(session.cursor_position().await.unwrap(), [123, 456]);
    for action in [
        ComputerAction::LeftMouseDown,
        ComputerAction::LeftMouseUp,
        ComputerAction::HoldKey {
            text: "shift".to_string(),
            duration: 0.2,
        },
        ComputerAction::LeftClickDrag {
            start_coordinate: [10, 10],
            coordinate: [40, 40],
            text: Some("shift".to_string()),
        },
    ] {
        session
            .perform(&action)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", action.name()));
    }
    // Zoom scales the region up to the display size.
    use base64::Engine;
    let zoomed = session.zoom([0, 0, 256, 192]).await.expect("zoom");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&zoomed.base64)
        .unwrap();
    assert_eq!(png_dimensions(&bytes).unwrap(), (1024, 768));

    // A terminal that writes one line of whatever is typed into it to a file.
    // Nothing in the path is a shell that could expand the typed text.
    let launched = client
        .exec(
            &state,
            &format!(
                "export DISPLAY={DESKTOP_DISPLAY}; \
                 if command -v xterm >/dev/null; then \
                   setsid xterm -geometry 80x24+0+0 -e sh -c 'head -n 1 > /tmp/typed.txt' \
                     >/dev/null 2>&1 </dev/null & \
                 elif command -v xfce4-terminal >/dev/null; then \
                   setsid xfce4-terminal --disable-server --geometry 80x24+0+0 \
                     -x sh -c 'head -n 1 > /tmp/typed.txt' >/dev/null 2>&1 </dev/null & \
                 else exit 3; fi; sleep 4"
            ),
            None,
            Some(30_000),
        )
        .await
        .expect("launch xterm");
    assert_ne!(
        launched.exit_code, 3,
        "the desktop template has no terminal to type into"
    );
    session
        .perform(&ComputerAction::LeftClick {
            coordinate: Some([100, 100]),
            text: None,
        })
        .await
        .expect("focus the terminal");
    let hostile = "-x $(touch /tmp/pwned) `id` ; echo \"q\" | tee 'a' && $HOME";
    session
        .perform(&ComputerAction::Type {
            text: hostile.to_string(),
        })
        .await
        .expect("type");
    session
        .perform(&ComputerAction::Key {
            text: "Return".to_string(),
            repeat: None,
        })
        .await
        .expect("key");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let typed = client
        .read_file(&state, "/tmp/typed.txt")
        .await
        .expect("read typed text");
    assert_eq!(typed.trim_end_matches('\n'), hostile);
    let pwned = client
        .exec(&state, "test -e /tmp/pwned", None, Some(30_000))
        .await
        .expect("check for injection");
    assert_ne!(pwned.exit_code, 0, "typed text was run by a shell");

    let frame = client
        .read_file_bytes(&state, "/tmp/everruns-computer/screen.png")
        .await
        .expect("read frame");
    assert_eq!(png_dimensions(&frame).unwrap(), (1024, 768));
    Box::new(session).release().await;
}
