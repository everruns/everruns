# TC001: Files Ls — List Session Files

## Description

Verify that `everruns files ls` lists files in a session's workspace and supports recursive and long output modes.

## Preconditions

- API server running (`just start-dev` or `just start-all`)
- An agent and session exist with files in the workspace

## Test Data

| Field | Value |
|-------|-------|
| Session | (created during test) |
| Test file 1 | `/hello.txt` with content `Hello, World!` |
| Test file 2 | `/src/main.rs` with content `fn main() {}` |

## Steps

1. Create an agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "Test Agent", "system_prompt": "Test"}' | jq -r '.id'
   ```
   Save as `$AGENT_ID`.

2. Create a session:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d "{\"agent_id\": \"$AGENT_ID\"}" | jq -r '.id'
   ```
   Save as `$SESSION_ID`.

3. Create test files:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/hello.txt" \
     -H "Content-Type: application/json" \
     -d '{"content": "Hello, World!", "encoding": "text"}'
   curl -s -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/fs/src/main.rs" \
     -H "Content-Type: application/json" \
     -d '{"content": "fn main() {}", "encoding": "text"}'
   ```

4. List files (short format):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files ls --session $SESSION_ID
   ```

5. List files (long format):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files ls --session $SESSION_ID -l
   ```

6. List files (recursive):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns files ls --session $SESSION_ID -r
   ```

7. List files (JSON output):
   ```bash
   EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_API_KEY=dev \
     everruns -o json files ls --session $SESSION_ID -r
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 4 output | Lists files at root level (may show `src/` directory and `hello.txt`) |
| Step 5 output | Shows SIZE, UPDATED, PATH columns |
| Step 6 output | Shows both `hello.txt` and `src/main.rs` |
| Step 7 output | Valid JSON with `entries` array containing file objects |
| File paths | Start with `/` |
| Directory entries | Suffixed with `/` in text mode |
