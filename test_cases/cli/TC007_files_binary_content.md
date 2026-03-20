# TC007: Files Push/Pull — Binary Content Handling

## Description

Verify that binary files (containing null bytes) are correctly handled with base64 encoding during push and pull operations.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist

## Steps

1. Create agent and session. Save `$SESSION_ID`.

2. Create a binary file locally:
   ```bash
   TMPDIR=$(mktemp -d)
   printf '\x00\x01\x02\x03\x04\x05' > "$TMPDIR/binary.bin"
   echo "text file" > "$TMPDIR/text.txt"
   ```

3. Push files:
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files push --session $SESSION_ID "$TMPDIR"
   ```

4. Verify remote encoding:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/binary.bin" | jq '.encoding'
   curl -s "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/text.txt" | jq '.encoding'
   ```

5. Pull to fresh directory:
   ```bash
   PULLDIR=$(mktemp -d)
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files pull --session $SESSION_ID "$PULLDIR"
   ```

6. Verify binary file integrity:
   ```bash
   xxd "$PULLDIR/binary.bin" | head -1
   diff "$TMPDIR/binary.bin" "$PULLDIR/binary.bin"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3 | Both files pushed successfully |
| Step 4 | `binary.bin` encoding is `base64`, `text.txt` is `text` |
| Step 5 | Both files pulled |
| Step 6 | Binary file identical after roundtrip (diff exits 0) |
