---
name: cloud-agent-smoke-tests
description: Run smoke tests in environments without Docker by setting up local PostgreSQL and Temporal services
---

# Cloud Agent Smoke Tests

Run smoke tests in Cloud Agent environments where Docker is not available.

## Usage

```bash
./.claude/skills/cloud-agent-smoke-tests/scripts/run-smoke-tests.sh
```

The script automatically:
1. Downloads Temporal CLI (if not present)
2. Starts Temporal dev server
3. Initializes PostgreSQL cluster
4. Creates database with UUIDv7 polyfill
5. Runs migrations
6. Starts the API server
7. Executes smoke tests
8. Cleans up on exit

## Requirements

- PostgreSQL 16+ binaries at `/usr/lib/postgresql/16/bin/`
- `postgres` system user
- Root access
- Rust toolchain with sqlx-cli

## Troubleshooting

**Run status shows "failed"**: Expected without `OPENAI_API_KEY`. The smoke test validates API operations work correctly.

**"cannot be run as root"**: Script handles this by running initdb as postgres user.

**"function uuidv7() does not exist"**: Script installs UUIDv7 polyfill automatically.
