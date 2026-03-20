# TC005: Files Push — Delete Remote Files Not Present Locally

## Description

Verify that `everruns files push --delete` removes remote files that are no longer present locally, after an initial sync establishes the baseline.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist

## Steps

1. Create agent and session. Save `$SESSION_ID`.

2. Create local directory with files:
   ```bash
   TMPDIR=$(mktemp -d)
   echo "keep me" > "$TMPDIR/keep.txt"
   echo "delete me" > "$TMPDIR/remove.txt"
   ```

3. Push both files:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

4. Delete local file:
   ```bash
   rm "$TMPDIR/remove.txt"
   ```

5. Push with --delete:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID --delete "$TMPDIR"
   ```

6. Verify remote state:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/keep.txt" | jq '.content'
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/remove.txt"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3 | Both files pushed (2 files) |
| Step 5 | Output includes `deleted: 1` |
| Step 6 | `keep.txt` still exists, `remove.txt` returns 404 |
