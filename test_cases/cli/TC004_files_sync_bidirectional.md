# TC004: Files Sync — Bidirectional Live Sync

## Description

Verify that `everruns files sync` performs bidirectional sync: initial reconciliation, local→remote on local changes, remote→local on remote changes, and graceful shutdown.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist

## Test Data

| Field | Value |
|-------|-------|
| Local file | `local.txt` with content `from local` |
| Remote file | `/remote.txt` with content `from remote` |
| Poll interval | 2 seconds (for faster test feedback) |

## Steps

1. Create agent and session. Save `$SESSION_ID`.

2. Create a remote file:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/remote.txt" \
     -H "Content-Type: application/json" \
     -d '{"content": "from remote", "encoding": "text"}'
   ```

3. Create local directory with a file:
   ```bash
   TMPDIR=$(mktemp -d)
   echo "from local" > "$TMPDIR/local.txt"
   ```

4. Start sync in background:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files sync --session $SESSION_ID --interval 2 --verbose "$TMPDIR" &
   SYNC_PID=$!
   sleep 5  # Wait for initial sync
   ```

5. Verify initial sync pulled remote file:
   ```bash
   cat "$TMPDIR/remote.txt"
   ```

6. Verify initial sync pushed local file:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/local.txt" | jq '.content'
   ```

7. Create new local file and wait for sync:
   ```bash
   echo "new file" > "$TMPDIR/new_local.txt"
   sleep 5
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/new_local.txt" | jq '.content'
   ```

8. Create new remote file and wait for sync:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/new_remote.txt" \
     -H "Content-Type: application/json" \
     -d '{"content": "new from remote", "encoding": "text"}'
   sleep 5
   cat "$TMPDIR/new_remote.txt"
   ```

9. Stop sync:
   ```bash
   kill $SYNC_PID
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 4 | Sync starts, prints `Initial sync...` and `Initial sync done: ↑1 ↓1` |
| Step 5 | `remote.txt` contains `from remote` |
| Step 6 | Remote `local.txt` contains `from local` |
| Step 7 | Remote `new_local.txt` synced |
| Step 8 | Local `new_remote.txt` contains `new from remote` |
| Step 9 | Clean shutdown with `Sync stopped. Total: ↑N ↓N` |
| Verbose output | Shows `↑` and `↓` for each file operation |
