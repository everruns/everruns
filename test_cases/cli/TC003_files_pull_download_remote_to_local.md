# TC003: Files Pull — Download Remote Files to Local

## Description

Verify that `everruns files pull` downloads session workspace files to a local directory, supports dry-run, and handles incremental sync.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist with files in the workspace

## Test Data

| Field | Value |
|-------|-------|
| Remote file 1 | `/config.yaml` with content `key: value` |
| Remote file 2 | `/src/app.rs` with content `fn app() {}` |
| Local dir | Empty temp directory |

## Steps

1. Create agent and session (as in TC001). Save `$SESSION_ID`.

2. Create remote files:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/config.yaml" \
     -H "Content-Type: application/json" \
     -d '{"content": "key: value", "encoding": "text"}'
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/src/app.rs" \
     -H "Content-Type: application/json" \
     -d '{"content": "fn app() {}", "encoding": "text"}'
   ```

3. Create empty local directory:
   ```bash
   TMPDIR=$(mktemp -d)
   ```

4. Dry-run pull:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files pull --session $SESSION_ID --dry-run "$TMPDIR"
   ```

5. Actual pull:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files pull --session $SESSION_ID "$TMPDIR"
   ```

6. Verify local files:
   ```bash
   cat "$TMPDIR/config.yaml"
   cat "$TMPDIR/src/app.rs"
   ```

7. Pull again (incremental — no changes):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files pull --session $SESSION_ID "$TMPDIR"
   ```

8. Pull with --delete after removing remote file:
   ```bash
   curl -s -X DELETE "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/config.yaml"
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files pull --session $SESSION_ID --delete "$TMPDIR"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 4 (dry-run) | Shows files that would be pulled, no actual download |
| Step 5 output | `Pulled: 2 files, deleted: 0, errors: 0` |
| Step 6 | `config.yaml` contains `key: value`, `src/app.rs` contains `fn app() {}` |
| Step 7 (re-pull) | `Pulled: 0 files` (incremental) |
| Step 8 (--delete) | `config.yaml` deleted from local dir |
| Parent dirs | `src/` directory auto-created |
| State file | `.everruns-sync/state.json` updated |
