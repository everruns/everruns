---
name: cloud-agent-smoke-tests
description: Run smoke tests in environments without Docker by setting up local PostgreSQL and Temporal services (project)
---

# Cloud Agent Smoke Tests

Run smoke tests in Cloud Agent environments where Docker is not available.

## Usage

```bash
export OPENAI_API_KEY=your-key
scripts/run-smoke-tests.sh
```

## Helper Scripts

- `scripts/run-smoke-tests.sh` - Main entry point, orchestrates full smoke test
- `scripts/setup-postgres.sh` - PostgreSQL cluster setup (prefers PostgreSQL 17, falls back to 16 with UUIDv7 polyfill)
- `scripts/setup-temporal.sh` - Temporal dev server setup
- `scripts/common.sh` - Shared utilities and configuration

## Requirements

- `OPENAI_API_KEY` environment variable
- Root access (for PostgreSQL setup)
- Rust toolchain with sqlx-cli
- PostgreSQL 16+ binaries (17 preferred, auto-installs if network available)

## Troubleshooting

**"OPENAI_API_KEY environment variable is not set"**: Export the key before running the script.

**"cannot be run as root"**: Script handles this by running initdb as postgres user.

**PostgreSQL 17 installation fails**: Script falls back to PostgreSQL 16 with UUIDv7 polyfill.
