// `everruns files pull` — one-shot download remote → local.

use crate::commands::files::remote::RemoteClient;
use crate::commands::files::state::{SyncState, content_hash, state_dir};
use crate::commands::files::sync_engine::{safe_local_path, scan_remote};
use crate::output::OutputFormat;
use anyhow::Result;
use std::path::PathBuf;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
    session_id: String,
    local_dir: String,
    delete: bool,
    dry_run: bool,
) -> Result<()> {
    let local_dir = PathBuf::from(&local_dir).canonicalize()?;
    let client = RemoteClient::new(api_url, api_key, &session_id);
    let sd = state_dir(&local_dir);
    let mut state = SyncState::load(&sd, &session_id)?;

    let remote_files = scan_remote(&client).await?;

    if !quiet && output.is_text() && !dry_run {
        eprintln!(
            "Pulling {} files from session {}...",
            remote_files.len(),
            session_id
        );
    }

    let mut downloaded = 0u32;
    let mut errors = 0u32;

    for (path, entry) in &remote_files {
        let remote_hash = entry
            .content_hash
            .as_deref()
            .or(entry.updated_at.as_deref())
            .unwrap_or("");

        // Check if changed since last sync
        let prev_hash = state.files.get(path).and_then(|s| s.remote_hash.as_deref());
        if prev_hash == Some(remote_hash) {
            continue;
        }

        if dry_run {
            println!("  ↓ {}", path);
            downloaded += 1;
            continue;
        }

        match client.read_file(&format!("/{}", path)).await {
            Ok(content) => {
                let bytes = RemoteClient::decode_content(&content)?;
                let local_path = safe_local_path(&local_dir, path)?;
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&local_path, &bytes)?;
                let hash = content_hash(&bytes);

                let file_state = state.files.entry(path.clone()).or_insert_with(|| {
                    crate::commands::files::state::FileSyncState {
                        local_hash: None,
                        remote_hash: None,
                        local_mtime: None,
                        remote_updated_at: None,
                    }
                });
                file_state.local_hash = Some(hash);
                file_state.remote_hash = Some(remote_hash.to_string());
                downloaded += 1;
            }
            Err(e) => {
                eprintln!("  ✗ {}: {}", path, e);
                errors += 1;
            }
        }
    }

    // Handle deletions
    let mut deleted = 0u32;
    if delete {
        let remote_paths: std::collections::HashSet<&str> =
            remote_files.keys().map(String::as_str).collect();
        // Check previously synced files that are no longer remote
        let prev_paths: Vec<String> = state.files.keys().cloned().collect();
        for path in prev_paths {
            if !remote_paths.contains(path.as_str()) {
                if let Ok(local_path) = safe_local_path(&local_dir, &path)
                    && local_path.exists()
                {
                    if dry_run {
                        println!("  🗑 local {}", path);
                    } else {
                        let _ = std::fs::remove_file(&local_path);
                    }
                    deleted += 1;
                }
                state.files.remove(&path);
            }
        }
    }

    if !dry_run {
        state.last_sync = Some(chrono::Utc::now().to_rfc3339());
        state.save(&sd)?;
    }

    if output.is_text() {
        if dry_run {
            println!("Would pull: {} files, delete: {}", downloaded, deleted);
        } else {
            println!(
                "Pulled: {} files, deleted: {}, errors: {}",
                downloaded, deleted, errors
            );
        }
    } else {
        output.print_value(&serde_json::json!({
            "downloaded": downloaded,
            "deleted": deleted,
            "errors": errors,
            "dry_run": dry_run,
        }));
    }

    Ok(())
}
