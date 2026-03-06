---
name: smoke-test
description: Run smoke tests to verify the Everruns system works correctly.
---

# Smoke Tests

Verify the system is working after changes.

## Quick Start

```bash
# Start server (no Docker needed, --no-watch for faster startup)
just start-dev --no-watch

# Run tests
cargo test
```

## Checklist

### 1. Start Server

```bash
just start-dev --no-watch  # In-memory, no Docker, fast startup
# or: just start-all --no-watch  # With PostgreSQL persistence
```

### 2. Health Check

```bash
curl -s http://localhost:9000/health | jq
# Expected: {"status": "ok", ...}
```

### 3. Run Tests

```bash
# All tests (unit + integration, includes tool calling tests)
cargo test --all-features

# Integration tests only (requires server running)
cargo test -p everruns-server --test integration_test -- --test-threads=1

# Durable engine tests
cargo test -p everruns-durable --lib
```

Integration tests cover: API endpoints, tool calling (OpenAI/Anthropic/LLMSim), events, sessions, capabilities.

### 4. UI Check

```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9300
# Expected: 200 or 307
```

### 5. API Endpoint Test

```bash
ORG="org_00000000000000000000000000000001"

# Create agent
curl -s -X POST "http://localhost:9300/api/v1/agents" \
  -H "Content-Type: application/json" \
  -d '{"name": "Test", "system_prompt": "You are helpful."}' | jq '.id'
```

### 6. Deployed Environment Smoke Tests

```bash
# Against dev.everruns.com (cloud agent with Doppler)
PLAYWRIGHT_CHROMIUM_PATH=/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome \
  doppler run -- npx playwright test --project smoke

# Against custom URL
SMOKE_BASE_URL=https://staging.everruns.com \
  npx playwright test --project smoke
```

Tests health, auth config, login page. Password-auth tests auto-skip on external auth deployments. See `specs/smoke-test-spec.md`.

## Full API Reference

See https://docs.everruns.com for complete API documentation.

## Troubleshooting

```bash
# Check ports
lsof -i :9300  # Caddy reverse proxy
lsof -i :9000  # API (direct)
lsof -i :9100  # UI (direct)

# View logs (DEV MODE runs in foreground)
# Ctrl+C to stop, then restart with: just start-dev
```
