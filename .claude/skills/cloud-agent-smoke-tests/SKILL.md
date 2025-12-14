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
- `scripts/setup-postgres.sh` - PostgreSQL 18 cluster setup (auto-installs from PGDG if needed)
- `scripts/setup-temporal.sh` - Temporal dev server setup
- `scripts/common.sh` - Shared utilities and configuration

## Requirements

- `OPENAI_API_KEY` environment variable
- Root access (for PostgreSQL setup)
- Rust toolchain with sqlx-cli
- Internet access (to install PostgreSQL 18 and Temporal CLI)

## Troubleshooting

**"OPENAI_API_KEY environment variable is not set"**: Export the key before running the script.

**"cannot be run as root"**: Script handles this by running initdb as postgres user.

**PostgreSQL installation fails**: Ensure internet access to apt.postgresql.org is available.
