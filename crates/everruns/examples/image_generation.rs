//! Ask an agent for an animated GIF and save it next to you.
//!
//! The agent runs on GPT-6.1 Sol and draws with GPT Image 2.6 Flare through
//! the `image_generation` capability. Generated images land in the session
//! workspace, here a real directory, so the GIF is a file you can open.
//!
//! ```text
//! cargo run -p everruns --features openai,images,local --example image_generation -- [OUT_DIR]
//! ```
// `LocalConfig` gives the session a real-disk workspace. It rides the legacy
// facade lane until hosted workspaces move to `SqliteHosting` (EVE-1298).
#![allow(deprecated)]

use std::path::PathBuf;

use everruns::{Agent, CapabilityRef, Engine, LocalConfig, OpenAI, WorkspacePolicy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("everruns-image-generation"));

    let agent = Agent::builder()
        .name("animator")
        .instructions(
            "You make short looping animations. Call generate_image with format gif, \
             then reply with one sentence about what moves.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .capability(CapabilityRef::with_config(
            "image_generation",
            serde_json::json!({
                "model": "gpt-image-2.6-flare",
                "complexity": "low",
                "gif_frames": 9,
                "gif_frame_ms": 160,
                // Hidden folders stay read-only under the workspace policy.
                "output_dir": "/workspace/images"
            }),
        ))
        .local(LocalConfig::new(&out))
        // The capability writes the GIF into the workspace.
        .workspace_policy(WorkspacePolicy::read_write())
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session
        .send_and_wait(
            "A paper lantern swaying on a string at dusk, warm glow, fireflies drifting past.",
        )
        .await?;
    if !turn.success {
        return Err(format!("turn failed: {}", turn.error.unwrap_or_default()).into());
    }
    println!("{}", turn.response);

    let gifs: Vec<PathBuf> = walk(&out)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "gif"))
        .collect();
    let Some(gif) = gifs.first() else {
        return Err(format!("no GIF under {}", out.display()).into());
    };
    let bytes = std::fs::read(gif)?;
    assert!(bytes.starts_with(b"GIF89a"), "not a GIF");
    println!("saved {} ({} KB)", gif.display(), bytes.len() / 1024);
    Ok(())
}

/// Every file under `root`.
fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}
