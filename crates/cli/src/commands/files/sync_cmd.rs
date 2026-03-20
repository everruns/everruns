// `everruns files sync` — long-running bidirectional file sync.
//
// Design Decision: Uses notify for local FS events + periodic remote polling.
// Design Decision: Graceful shutdown on Ctrl+C with stats summary.

use crate::commands::files::remote::RemoteClient;
use crate::commands::files::state::{SyncState, state_dir};
use crate::commands::files::sync_engine::{Conflict, reconcile};
use anyhow::Result;
use notify::{Event, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    api_url: &str,
    api_key: &str,
    quiet: bool,
    session_id: String,
    local_dir: String,
    interval: u64,
    conflict: String,
    exclude: Vec<String>,
    no_gitignore: bool,
    dry_run: bool,
    delete: bool,
    verbose: bool,
) -> Result<()> {
    let local_dir = PathBuf::from(&local_dir).canonicalize()?;
    let client = RemoteClient::new(api_url, api_key, &session_id);
    let conflict_strategy = Conflict::parse(&conflict);
    let sd = state_dir(&local_dir);
    let mut state = SyncState::load(&sd, &session_id)?;

    if !quiet {
        eprintln!(
            "Syncing {} ↔ session {} (poll {}s, conflict: {})",
            local_dir.display(),
            session_id,
            interval,
            conflict,
        );
        if dry_run {
            eprintln!("(dry-run mode)");
        }
    }

    // Initial full reconciliation
    if !quiet {
        eprintln!("Initial sync...");
    }
    let stats = reconcile(
        &client,
        &local_dir,
        &mut state,
        conflict_strategy,
        no_gitignore,
        &exclude,
        dry_run,
        delete,
        verbose,
    )
    .await?;
    if !quiet {
        eprintln!("Initial sync done: {}", stats);
    }

    // Set up local filesystem watcher
    let local_changed = Arc::new(AtomicBool::new(false));
    let changed_flag = local_changed.clone();

    let local_dir_clone = local_dir.clone();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            // Ignore changes in .everruns-sync directory
            let dominated_by_sync = event
                .paths
                .iter()
                .all(|p| p.starts_with(local_dir_clone.join(".everruns-sync")));
            if !dominated_by_sync {
                changed_flag.store(true, Ordering::Relaxed);
            }
        }
    })?;

    watcher.watch(&local_dir, RecursiveMode::Recursive)?;

    // Shutdown signal
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        r.store(false, Ordering::Relaxed);
    });

    // Main sync loop
    let poll_dur = Duration::from_secs(interval);
    let mut total_up: u64 = 0;
    let mut total_down: u64 = 0;

    while running.load(Ordering::Relaxed) {
        tokio::time::sleep(poll_dur).await;

        if !running.load(Ordering::Relaxed) {
            break;
        }

        // Always poll remote; also sync if local changed
        let _local_dirty = local_changed.swap(false, Ordering::Relaxed);

        let stats = reconcile(
            &client,
            &local_dir,
            &mut state,
            conflict_strategy,
            no_gitignore,
            &exclude,
            dry_run,
            delete,
            verbose,
        )
        .await;

        match stats {
            Ok(s) => {
                total_up += s.uploaded as u64;
                total_down += s.downloaded as u64;
                let has_activity =
                    s.uploaded > 0 || s.downloaded > 0 || s.conflicts > 0 || s.errors > 0;
                if has_activity && !quiet {
                    eprintln!("{}", s);
                }
            }
            Err(e) => {
                eprintln!("Sync error: {}", e);
            }
        }
    }

    if !quiet {
        eprintln!("\nSync stopped. Total: ↑{} ↓{}", total_up, total_down);
    }

    Ok(())
}
