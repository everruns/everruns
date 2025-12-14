---
name: cloud-agent-smoke-tests
description: Run smoke tests in environments without Docker by setting up local PostgreSQL and Temporal services
---

# Cloud Agent Smoke Tests

Run smoke tests in Cloud Agent environments where Docker is not available.

## Usage

```bash
scripts/run-smoke-tests.sh
```

## Helper Scripts

- `scripts/run-smoke-tests.sh` - Main entry point, orchestrates full smoke test
- `scripts/setup-postgres.sh` - PostgreSQL cluster setup functions
- `scripts/setup-temporal.sh` - Temporal dev server setup functions
- `scripts/common.sh` - Shared utilities and configuration

## Requirements

- PostgreSQL 16+ binaries at `/usr/lib/postgresql/16/bin/`
- `postgres` system user
- Root access
- Rust toolchain with sqlx-cli

## Troubleshooting

**Run status shows "failed"**: Expected without `OPENAI_API_KEY`. The smoke test validates API operations work correctly.

**"cannot be run as root"**: Script handles this by running initdb as postgres user.

**"function uuidv7() does not exist"**: Script installs UUIDv7 polyfill automatically.
