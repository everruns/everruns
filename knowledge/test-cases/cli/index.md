# CLI test cases

* [TC001: Files Ls — List Session Files](TC001_files_ls_list_session_files.md) - Verify that `everruns files ls` lists files in a session's workspace and supports recursive and long output modes.
* [TC002: Files Push — Upload Local Files to Remote](TC002_files_push_upload_local_to_remote.md) - Verify that `everruns files push` uploads local files to a session's workspace, supports dry-run, and tracks state for incremental sync.
* [TC003: Files Pull — Download Remote Files to Local](TC003_files_pull_download_remote_to_local.md) - Verify that `everruns files pull` downloads session workspace files to a local directory, supports dry-run, and handles incremental sync.
* [TC004: Files Sync — Bidirectional Live Sync](TC004_files_sync_bidirectional.md) - Verify that `everruns files sync` performs bidirectional sync: initial reconciliation, local→remote on local changes, remote→local on remote changes, and graceful shutdown.
* [TC005: Files Push — Delete Remote Files Not Present Locally](TC005_files_push_with_delete.md) - Verify that `everruns files push --delete` removes remote files that are no longer present locally, after an initial sync establishes the baseline.
* [TC006: Files Sync — Conflict Resolution](TC006_files_sync_conflict_resolution.md) - Verify that `everruns files sync` correctly detects and resolves conflicts when both local and remote versions of a file change between sync cycles.
* [TC007: Files Push/Pull — Binary Content Handling](TC007_files_binary_content.md) - Verify that binary files (containing null bytes) are correctly handled with base64 encoding during push and pull operations.
