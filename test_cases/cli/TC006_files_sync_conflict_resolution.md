# TC006: Files Sync — Conflict Resolution

## Description

Verify that `everruns files sync` correctly detects and resolves conflicts when both local and remote versions of a file change between sync cycles.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist

## Steps

1. Create agent and session. Save `$SESSION_ID`.

2. Create initial file locally and push:
   ```bash
   TMPDIR=$(mktemp -d)
   echo "version 1" > "$TMPDIR/conflict.txt"
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

3. Modify both local and remote:
   ```bash
   echo "local version 2" > "$TMPDIR/conflict.txt"
   curl -s -X PUT "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/conflict.txt" \
     -H "Content-Type: application/json" \
     -d '{"content": "remote version 2", "encoding": "text"}'
   ```

4. Sync with local-wins strategy:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files sync --session $SESSION_ID --conflict local-wins --verbose "$TMPDIR" &
   SYNC_PID=$!
   sleep 5
   kill $SYNC_PID
   ```

5. Check remote file:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/conflict.txt" | jq '.content'
   ```

6. Repeat steps 2-4 with `--conflict remote-wins`:
   ```bash
   # ... same setup, then sync with remote-wins
   ```

7. Check local file:
   ```bash
   cat "$TMPDIR/conflict.txt"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 4 output | Shows `! conflict: conflict.txt (local wins)` |
| Step 5 (local-wins) | Remote contains `local version 2` |
| Step 6/7 (remote-wins) | Local contains `remote version 2` |
| Stats | `conflicts:1` in sync output |
