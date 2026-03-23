//! Live Sprites API integration tests.
//!
//! These tests hit the real Sprites API and are gated behind:
//! - Feature flag: `sprites-live-tests`
//! - Environment variable: `SPRITES_API_TOKEN`
//!
//! Run locally:
//!   SPRITES_API_TOKEN=<token> cargo test -p everruns-integrations-sprites \
//!       --features sprites-live-tests --test live_api_test -- --test-threads=1
//!
//! Cleanup guarantee: Each test uses a `SpriteGuard` that deletes the sprite
//! on drop (both success and panic paths).

#![cfg(feature = "sprites-live-tests")]

use everruns_integrations_sprites::client::SpritesClient;
use serde_json::json;

// ============================================================================
// SpriteGuard — RAII cleanup for sprites
// ============================================================================

struct SpriteGuard {
    sprite_name: String,
}

impl SpriteGuard {
    fn new(sprite_name: String) -> Self {
        Self { sprite_name }
    }
}

impl Drop for SpriteGuard {
    fn drop(&mut self) {
        let name = self.sprite_name.clone();
        let Some(api_token) = get_api_token() else {
            eprintln!("[cleanup] No API token, cannot delete sprite {name}");
            return;
        };
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("cleanup runtime");
            let client = SpritesClient::new(api_token);
            rt.block_on(async {
                eprintln!("[cleanup] Deleting sprite {name}");
                match client.delete_sprite(&name).await {
                    Ok(()) => eprintln!("[cleanup] Sprite {name} deleted"),
                    Err(e) => eprintln!("[cleanup] Failed to delete sprite {name}: {e}"),
                }
            });
        });
        let _ = handle.join();
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn get_api_token() -> Option<String> {
    std::env::var("SPRITES_API_TOKEN")
        .ok()
        .filter(|k| !k.is_empty())
}

macro_rules! require_api_token {
    () => {
        match get_api_token() {
            Some(token) => token,
            None => {
                eprintln!("[skip] SPRITES_API_TOKEN not set, skipping live test");
                return;
            }
        }
    };
}

/// Create a sprite with a unique test label and return client + guard.
async fn create_test_sprite(api_token: String, label: &str) -> (SpritesClient, SpriteGuard) {
    let client = SpritesClient::new(api_token);
    let name = format!("everruns-test-{label}");

    let info = client
        .create_sprite(
            &name,
            json!({
                "guest": {"cpus": 1, "memory_mb": 256},
                "metadata": {"everruns-test": label}
            }),
        )
        .await
        .expect("Failed to create sprite");

    assert_eq!(info.name, name);
    eprintln!("[test] Created sprite {name}");

    let guard = SpriteGuard::new(name);
    (client, guard)
}

// ============================================================================
// Tests
// ============================================================================

/// Full lifecycle: create → exec → file roundtrip → checkpoint → delete.
#[tokio::test]
async fn test_live_sprite_lifecycle() {
    let api_token = require_api_token!();
    let (client, guard) = create_test_sprite(api_token, "lifecycle").await;
    let name = &guard.sprite_name;

    // Exec: simple echo
    let result = client
        .exec(name, "echo hello-everruns", None)
        .await
        .expect("exec failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("hello-everruns"),
        "Unexpected output: {}",
        result.stdout
    );

    // File write + read roundtrip
    let content = b"print('hello from everruns live test')\n";
    client
        .write_file(name, "/tmp/test_live.py", content)
        .await
        .expect("file write failed");

    let downloaded = client
        .read_file(name, "/tmp/test_live.py")
        .await
        .expect("file read failed");
    assert_eq!(
        downloaded, content,
        "Downloaded content doesn't match uploaded"
    );

    // Checkpoint
    let cp = client
        .create_checkpoint(name)
        .await
        .expect("checkpoint failed");
    assert!(!cp.id.is_empty(), "Checkpoint ID should not be empty");

    // Verify sprite info
    let info = client.get_sprite(name).await.expect("get_sprite failed");
    assert_eq!(info.name, *name);

    // Explicit delete
    client
        .delete_sprite(name)
        .await
        .expect("delete_sprite failed");
}

/// Exec with nonzero exit code.
#[tokio::test]
async fn test_live_exec_exit_code() {
    let api_token = require_api_token!();
    let (client, guard) = create_test_sprite(api_token, "exit-code").await;
    let name = &guard.sprite_name;

    let result = client
        .exec(name, "exit 42", None)
        .await
        .expect("exec failed");
    assert_ne!(result.exit_code, 0, "Expected nonzero exit code, got 0");
}

/// Checkpoint and restore.
#[tokio::test]
async fn test_live_checkpoint_restore() {
    let api_token = require_api_token!();
    let (client, guard) = create_test_sprite(api_token, "checkpoint").await;
    let name = &guard.sprite_name;

    // Write a file
    client
        .write_file(name, "/tmp/before.txt", b"original")
        .await
        .expect("write failed");

    // Checkpoint
    let cp = client
        .create_checkpoint(name)
        .await
        .expect("checkpoint failed");

    // Overwrite the file
    client
        .write_file(name, "/tmp/before.txt", b"modified")
        .await
        .expect("overwrite failed");

    // Restore
    client
        .restore_checkpoint(name, &cp.id)
        .await
        .expect("restore failed");

    // Verify file was restored
    let content = client
        .read_file(name, "/tmp/before.txt")
        .await
        .expect("read failed");
    assert_eq!(
        String::from_utf8_lossy(&content),
        "original",
        "File should be restored to checkpoint state"
    );
}
