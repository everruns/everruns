// `everruns files push` — one-shot upload local → remote.

use crate::commands::files::remote::RemoteClient;
use crate::commands::files::state::{SyncState, state_dir};
use crate::commands::files::sync_engine::scan_local;
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

    let local_files = scan_local(&local_dir, false, &[])?;

    if !quiet && output.is_text() && !dry_run {
        eprintln!(
            "Pushing {} files to session {}...",
            local_files.len(),
            session_id
        );
    }

    let mut uploaded = 0u32;
    let mut errors = 0u32;

    for (path, (content, hash)) in &local_files {
        // Check if changed since last sync
        let prev_hash = state.files.get(path).and_then(|s| s.local_hash.as_deref());
        if prev_hash == Some(hash.as_str()) {
            continue;
        }

        if dry_run {
            println!("  ↑ {}", path);
            uploaded += 1;
            continue;
        }

        match client
            .write_file(&format!("/{}", path), content, true)
            .await
        {
            Ok(()) => {
                uploaded += 1;
                let entry = state.files.entry(path.clone()).or_insert_with(|| {
                    crate::commands::files::state::FileSyncState {
                        local_hash: None,
                        remote_hash: None,
                        local_mtime: None,
                        remote_updated_at: None,
                    }
                });
                entry.local_hash = Some(hash.clone());
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
        let remote_files = crate::commands::files::sync_engine::scan_remote(&client).await?;
        for path in remote_files.keys() {
            if !local_files.contains_key(path) {
                if dry_run {
                    println!("  🗑 remote {}", path);
                } else {
                    let _ = client.delete(&format!("/{}", path), false).await;
                }
                deleted += 1;
                state.files.remove(path);
            }
        }
    }

    if !dry_run {
        state.last_sync = Some(chrono::Utc::now().to_rfc3339());
        state.save(&sd)?;
    }

    if output.is_text() {
        if dry_run {
            println!("Would push: {} files, delete: {}", uploaded, deleted);
        } else {
            println!(
                "Pushed: {} files, deleted: {}, errors: {}",
                uploaded, deleted, errors
            );
        }
    } else {
        output.print_value(&serde_json::json!({
            "uploaded": uploaded,
            "deleted": deleted,
            "errors": errors,
            "dry_run": dry_run,
        }));
    }

    Ok(())
}
