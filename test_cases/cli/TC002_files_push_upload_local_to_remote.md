# TC002: Files Push — Upload Local Files to Remote

## Description

Verify that `everruns files push` uploads local files to a session's workspace, supports dry-run, and tracks state for incremental sync.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist

## Test Data

| Field | Value |
|-------|-------|
| Local dir | Temp directory with test files |
| File 1 | `hello.txt` with content `Hello from push!` |
| File 2 | `src/lib.rs` with content `pub fn greet() {}` |

## Steps

1. Create agent and session (as in TC001). Save `$SESSION_ID`.

2. Create local test directory:
   ```bash
   TMPDIR=$(mktemp -d)
   echo "Hello from push!" > "$TMPDIR/hello.txt"
   mkdir -p "$TMPDIR/src"
   echo "pub fn greet() {}" > "$TMPDIR/src/lib.rs"
   ```

3. Dry-run push:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID --dry-run "$TMPDIR"
   ```

4. Actual push:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

5. Verify remote files:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/hello.txt" | jq '.content'
   ```

6. Push again (no changes — should be incremental):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

7. Modify a file and push again:
   ```bash
   echo "Updated content" > "$TMPDIR/hello.txt"
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

8. JSON output push:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns -o json files push --session $SESSION_ID "$TMPDIR"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3 (dry-run) | Shows files that would be pushed, no actual upload |
| Step 4 output | `Pushed: 2 files, deleted: 0, errors: 0` |
| Step 5 | Content matches `Hello from push!` |
| Step 6 (re-push) | `Pushed: 0 files` (incremental — already synced) |
| Step 7 (after modify) | `Pushed: 1 files` (only changed file) |
| Step 8 (JSON) | Valid JSON with `uploaded`, `deleted`, `errors` fields |
| State file | `.everruns-sync/state.json` created in local dir |
