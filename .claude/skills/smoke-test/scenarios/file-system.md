# File System Scenario

Tests for the session-level virtual filesystem functionality.

This scenario covers two testing approaches:
1. **REST API Tests** - Direct HTTP calls to the filesystem endpoints
2. **Capability Tool Tests** - Testing tools via agent workflow (LLM tool calling)

## Prerequisites

- API server running at `http://localhost:9000`
- Temporal worker running (for capability tests)
- An existing agent and session (from main smoke tests)

```bash
# Set up variables (reuse from main smoke tests)
# AGENT_ID=<your-agent-id>
# SESSION_ID=<your-session-id>
BASE_URL="http://localhost:9000"
FS_URL="$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/fs"
```

---

## Capability Tool Tests

These tests verify the FileSystem capability tools work correctly through the agent workflow.

### Setup: Create Agent with FileSystem Capability

```bash
# Create agent with file_system capability
AGENT=$(curl -s -X POST "$BASE_URL/v1/agents" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "FileSystem Test Agent",
    "system_prompt": "You are a helpful assistant with file system access.",
    "description": "Agent for testing FileSystem capability"
  }')
AGENT_ID=$(echo $AGENT | jq -r '.id')

# Set the file_system capability
curl -s -X PUT "$BASE_URL/v1/agents/$AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["file_system"]}' | jq

# Create a session
SESSION=$(curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "FileSystem Test Session"}')
SESSION_ID=$(echo $SESSION | jq -r '.id')
echo "Agent: $AGENT_ID, Session: $SESSION_ID"
```

### Test: write_file Tool

```bash
# Send message to trigger write_file tool
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Please create a file at /hello.txt with the content \"Hello from FileSystem capability!\""}]
    }
  }'

# Wait for workflow
sleep 10

# Check for assistant response with tool usage
curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "assistant")'
```
Expected: Assistant message indicating file was created, with tool_call content part for write_file

### Test: read_file Tool

```bash
# Ask agent to read the file
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Read the content of /hello.txt"}]
    }
  }'

sleep 10

# Check response includes the file content
curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[-1]'
```
Expected: Assistant reads and reports file content "Hello from FileSystem capability!"

### Test: list_directory Tool

```bash
# Ask agent to list files
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "List all files in the root directory /"}]
    }
  }'

sleep 10

curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[-1]'
```
Expected: Assistant lists /hello.txt

### Test: stat_file Tool

```bash
# Ask for file metadata
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Get the metadata for /hello.txt including size and timestamps"}]
    }
  }'

sleep 10

curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[-1]'
```
Expected: Assistant reports file exists, size, creation/update times

### Test: grep_files Tool

```bash
# First create more files
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Create /notes/todo.txt with content \"Buy groceries\nCall mom\nFinish project\" and /notes/done.txt with \"Completed: Clean room\""}]
    }
  }'

sleep 15

# Search for pattern
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Search all files for the word \"project\""}]
    }
  }'

sleep 10

curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[-1]'
```
Expected: Assistant finds "Finish project" in /notes/todo.txt

### Test: delete_file Tool

```bash
# Delete a file
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "Delete the file /notes/done.txt"}]
    }
  }'

sleep 10

# Verify deletion via list
curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [{"type": "text", "text": "List files in /notes directory"}]
    }
  }'

sleep 10

curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[-1]'
```
Expected: Only todo.txt remains in /notes

---

## REST API Tests

### 1. List Root Directory (Empty)
```bash
curl -s "$FS_URL" | jq
```
Expected: `{"data": []}`

### 2. Create a File
```bash
curl -s -X POST "$FS_URL/hello.txt" \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello, World!", "encoding": "text"}' | jq
```
Expected: File object with `path: "/hello.txt"`, `is_directory: false`

### 3. Read File Content
```bash
curl -s "$FS_URL/hello.txt" | jq '.content'
```
Expected: `"Hello, World!"`

### 4. Get File Stat
```bash
curl -s -X POST "$FS_URL/_/stat" \
  -H "Content-Type: application/json" \
  -d '{"path": "/hello.txt"}' | jq
```
Expected: FileStat with `path`, `name`, `size_bytes`, `is_directory: false`

### 5. Update File Content
```bash
curl -s -X PUT "$FS_URL/hello.txt" \
  -H "Content-Type: application/json" \
  -d '{"content": "Updated content"}' | jq '.content'
```
Expected: `"Updated content"`

### 6. Create Directory
```bash
curl -s -X POST "$FS_URL/docs" \
  -H "Content-Type: application/json" \
  -d '{"is_directory": true}' | jq
```
Expected: File object with `path: "/docs"`, `is_directory: true`

### 7. Create Nested File (Auto-creates Parent Dirs)
```bash
curl -s -X POST "$FS_URL/src/main.rs" \
  -H "Content-Type: application/json" \
  -d '{"content": "fn main() {}", "encoding": "text"}' | jq '.path'
```
Expected: `"/src/main.rs"`

### 8. List Directory
```bash
curl -s "$FS_URL/src" | jq '.data[].name'
```
Expected: `"main.rs"`

### 9. List All Files (Recursive)
```bash
curl -s "$FS_URL?recursive=true" | jq '.data | length'
```
Expected: At least 3 files (hello.txt, docs, src/main.rs)

### 10. Copy File
```bash
curl -s -X POST "$FS_URL/_/copy" \
  -H "Content-Type: application/json" \
  -d '{"src_path": "/hello.txt", "dst_path": "/hello-backup.txt"}' | jq '.path'
```
Expected: `"/hello-backup.txt"`

### 11. Move/Rename File
```bash
curl -s -X POST "$FS_URL/_/move" \
  -H "Content-Type: application/json" \
  -d '{"src_path": "/hello-backup.txt", "dst_path": "/renamed.txt"}' | jq '.path'
```
Expected: `"/renamed.txt"`

### 12. Grep Search
```bash
curl -s -X POST "$FS_URL/_/grep" \
  -H "Content-Type: application/json" \
  -d '{"pattern": "main"}' | jq
```
Expected: Results with matches in `/src/main.rs`

### 13. Delete File
```bash
curl -s -X DELETE "$FS_URL/renamed.txt" | jq
```
Expected: `{"deleted": true}`

### 14. Delete Directory (Non-recursive - should fail if not empty)
```bash
curl -s -X DELETE "$FS_URL/src" 2>&1
```
Expected: 400 error "Directory is not empty"

### 15. Delete Directory (Recursive)
```bash
curl -s -X DELETE "$FS_URL/src?recursive=true" | jq
```
Expected: `{"deleted": true}`

### 16. Binary File (Base64)
```bash
# Create binary file (PNG magic bytes as base64)
curl -s -X POST "$FS_URL/test.bin" \
  -H "Content-Type: application/json" \
  -d '{"content": "iVBORw0KGgo=", "encoding": "base64"}' | jq '.encoding'
```
Expected: `"base64"`

### 17. Readonly File
```bash
# Create readonly file
curl -s -X POST "$FS_URL/readonly.txt" \
  -H "Content-Type: application/json" \
  -d '{"content": "Cannot modify", "is_readonly": true}' | jq '.is_readonly'
```
Expected: `true`

```bash
# Try to update readonly file (should fail)
curl -s -X PUT "$FS_URL/readonly.txt" \
  -H "Content-Type: application/json" \
  -d '{"content": "Modified"}' 2>&1
```
Expected: 400 error about readonly file

## UI Tests

### 1. Session Page with File System Tab
```bash
curl -s -o /dev/null -w "%{http_code}" "http://localhost:9100/agents/$AGENT_ID/sessions/$SESSION_ID"
```
Expected: 200

### 2. Verify File System Tab Renders (manual)
Navigate to session page and verify:
- [ ] "File System" tab is visible next to "Chat" tab
- [ ] Clicking "File System" tab shows file browser
- [ ] Files created via API appear in the browser
- [ ] Can create new files using the "+" button
- [ ] Can create directories using the folder button
- [ ] Can delete files using the trash icon
- [ ] Clicking a file opens it in the file viewer
- [ ] Can edit and save file content

## Cleanup
```bash
# Delete remaining test files
curl -s -X DELETE "$FS_URL/hello.txt"
curl -s -X DELETE "$FS_URL/docs?recursive=true"
curl -s -X DELETE "$FS_URL/test.bin"
curl -s -X DELETE "$FS_URL/readonly.txt"
```

## Quick Automated Test Script

Run this script to test the basic file system flow:

```bash
#!/bin/bash
set -e

BASE_URL="${BASE_URL:-http://localhost:9000}"
FS_URL="$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/fs"

echo "Testing file system at $FS_URL"

# Create file
echo -n "Creating file... "
RESULT=$(curl -s -X POST "$FS_URL/test-file.txt" \
  -H "Content-Type: application/json" \
  -d '{"content": "test content"}')
echo "$RESULT" | jq -r '.path' && echo "OK"

# Read file
echo -n "Reading file... "
CONTENT=$(curl -s "$FS_URL/test-file.txt" | jq -r '.content')
[ "$CONTENT" = "test content" ] && echo "OK" || echo "FAIL: $CONTENT"

# List files
echo -n "Listing files... "
COUNT=$(curl -s "$FS_URL" | jq '.data | length')
[ "$COUNT" -ge 1 ] && echo "OK ($COUNT files)" || echo "FAIL"

# Delete file
echo -n "Deleting file... "
curl -s -X DELETE "$FS_URL/test-file.txt" | jq -r '.deleted' | grep -q true && echo "OK" || echo "FAIL"

echo "All tests passed!"
```
