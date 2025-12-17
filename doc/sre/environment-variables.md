# Everruns Environment Variables

This document describes the environment variables used to configure the Everruns API.

## API Configuration

### API_PREFIX

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Empty (no prefix) |
| **Example** | `/api` |

**Description:**

Optional prefix for all API routes. When set, all v1 API endpoints will be prefixed with this value.

**Behavior:**

- **Empty/Unset (default):** Routes are served at their standard paths (e.g., `/v1/agents`, `/v1/agents/{id}/sessions`)
- **Set to `/api`:** Routes are served with the prefix (e.g., `/api/v1/agents`, `/api/v1/agents/{id}/sessions`)

**Note:** The following routes are NOT affected by `API_PREFIX`:
- `/health` - Health check endpoint (for load balancers)
- `/swagger-ui` - OpenAPI documentation UI
- `/api-doc/openapi.json` - OpenAPI specification

**Use cases:**

1. **Reverse proxy configuration:** When running behind a reverse proxy that routes `/api/*` to the Everruns API
2. **Path-based routing:** When multiple services share the same domain with different path prefixes
3. **API Gateway integration:** When the API gateway expects a specific path prefix

**Example configuration:**

```bash
# No prefix (default) - routes at /v1/agents
# API_PREFIX=

# With /api prefix - routes at /api/v1/agents
API_PREFIX=/api
```

**Verification:**

After setting `API_PREFIX`, verify the configuration:

```bash
# Health check (always at root)
curl http://localhost:9000/health

# API endpoint with prefix
curl http://localhost:9000/api/v1/agents

# Without prefix (default)
curl http://localhost:9000/v1/agents
```

---

## Database Configuration

### DATABASE_URL

| Property | Value |
|----------|-------|
| **Required** | Yes |
| **Format** | PostgreSQL connection string |

PostgreSQL connection URL for the Everruns database.

**Example:**
```bash
DATABASE_URL=postgres://everruns:everruns@localhost:5432/everruns
```

---

## Secrets Encryption

### SECRETS_ENCRYPTION_KEY

| Property | Value |
|----------|-------|
| **Required** | No (but recommended for production) |
| **Format** | `key_id:base64_key` |

Primary encryption key for sensitive data (API keys stored in database). See [Encryption Key Rotation](runbooks/encryption-key-rotation.md) for key management.

### SECRETS_ENCRYPTION_KEY_PREVIOUS

| Property | Value |
|----------|-------|
| **Required** | No |
| **Format** | `key_id:base64_key` |

Previous encryption key for key rotation. Allows decryption of data encrypted with old key while new data uses the primary key.

---

## Agent Runner Configuration

### AGENT_RUNNER_MODE

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `in-process` |
| **Options** | `in-process`, `temporal` |

Controls how agent runs are executed:

- **`in-process`:** Executes agent loops directly in the API process (simpler, good for development)
- **`temporal`:** Executes via Temporal for durability and reliability (recommended for production)

### TEMPORAL_ADDRESS

| Property | Value |
|----------|-------|
| **Required** | Only when `AGENT_RUNNER_MODE=temporal` |
| **Default** | `localhost:7233` |

Temporal server address.

### TEMPORAL_NAMESPACE

| Property | Value |
|----------|-------|
| **Required** | Only when `AGENT_RUNNER_MODE=temporal` |
| **Default** | `default` |

Temporal namespace for workflow execution.

### TEMPORAL_TASK_QUEUE

| Property | Value |
|----------|-------|
| **Required** | Only when `AGENT_RUNNER_MODE=temporal` |
| **Default** | `everruns-agent-runs` |

Temporal task queue name for agent run workflows.
