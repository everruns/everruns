# Everruns Server (Control Plane)

The control plane component of Everruns. Provides REST API and gRPC server for managing AI agents, sessions, and runs. Workers connect to this server via gRPC for all database operations.

## Features

- ✅ Agent CRUD operations
- ✅ Agent versioning (immutable snapshots)
- ✅ Thread and message management
- ✅ Run execution tracking
- ✅ OpenAPI/Swagger documentation
- ✅ Multi-tenant isolation

## Quick Start

### 1. Start Services

```bash
just start-all  # Start PostgreSQL, API, Worker, UI
```

Migrations are applied automatically on server startup.

The API will be available at `http://localhost:9000`

### 3. View API Documentation

Open your browser to:
- **Swagger UI**: http://localhost:9000/swagger-ui/
- **OpenAPI Spec**: http://localhost:9000/api-doc/openapi.json

## Examples

### Run the Example

```bash
# Make sure the API server is running first
cargo run --example create_agent
```

This will:
1. Create a new agent
2. Create an agent version
3. Retrieve the agent
4. List all agents

### Example Output

```
🔑 Using tenant_id: 01234567-89ab-cdef-0123-456789abcdef

📝 Creating agent...
✅ Created agent:
   ID: 01234567-89ab-cdef-0123-456789abcdef
   Name: My First Agent
   Status: Active
   Created at: 2025-12-13T06:30:00Z

📦 Creating agent version...
✅ Created agent version:
   Version: 1
   Agent ID: 01234567-89ab-cdef-0123-456789abcdef

🎉 Example completed successfully!
```

## Integration Tests

### Prerequisites

1. Start the API server: `just api`
2. Ensure the database is clean (or use a test database)

### Run Tests

```bash
# Run all integration tests (requires API + Worker running)
cargo test -p everruns-server --test integration_test -- --test-threads=1

# Run a specific test
cargo test -p everruns-server --test integration_test test_full_agent_session_workflow -- --test-threads=1
```

### Test Coverage

- ✅ Full agent workflow (create, update, version)
- ✅ Thread and message operations
- ✅ Run creation and retrieval
- ✅ Health endpoint
- ✅ OpenAPI spec validation

## API Endpoints

### Agents

- `POST /v1/agents` - Create agent
- `GET /v1/agents?tenant_id=<uuid>` - List agents
- `GET /v1/agents/:id?tenant_id=<uuid>` - Get agent
- `PATCH /v1/agents/:id?tenant_id=<uuid>` - Update agent
- `POST /v1/agents/:id/versions` - Create version
- `GET /v1/agents/:id/versions` - List versions
- `GET /v1/agents/:id/versions/:version` - Get version

### Threads

- `POST /v1/threads` - Create thread
- `GET /v1/threads/:id?tenant_id=<uuid>` - Get thread
- `POST /v1/threads/:id/messages` - Add message
- `GET /v1/threads/:id/messages` - List messages

### Runs

- `POST /v1/runs` - Create run
- `GET /v1/runs/:id?tenant_id=<uuid>` - Get run

### System

- `GET /health` - Health check
- `GET /swagger-ui/` - Interactive API documentation
- `GET /api-doc/openapi.json` - OpenAPI specification

## Development

### Build

```bash
cargo build -p everrun-api
```

### Run with Custom Port

```bash
# Currently fixed at 9000, but can be made configurable
cargo run -p everrun-api
```

### Format Code

```bash
cargo fmt -p everrun-api
```

### Lint

```bash
cargo clippy -p everrun-api -- -D warnings
```

## Architecture

- **Framework**: Axum (async Rust web framework)
- **Database**: PostgreSQL 17 with custom UUIDv7 function
- **Documentation**: utoipa + Swagger UI
- **Validation**: Multi-tenant isolation at DB level
- **Error Handling**: Structured error responses

## Configuration

Currently configured via environment variables:

- `DATABASE_URL` - PostgreSQL connection string (required)

Default: `postgres://everrun:everrun@localhost:5432/everrun`

## Testing with cURL

### Create an Agent

```bash
TENANT_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')

curl -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d "{
    \"tenant_id\": \"$TENANT_ID\",
    \"name\": \"Test Agent\",
    \"description\": \"A test agent\",
    \"default_model_id\": \"gpt-5.1\"
  }" | jq
```

### List Agents

```bash
curl "http://localhost:9000/v1/agents?tenant_id=$TENANT_ID" | jq
```

## Troubleshooting

### Database Connection Errors

Make sure PostgreSQL is running:
```bash
just start
```

### Migration Errors

Reset the database and restart (migrations auto-apply):
```bash
just reset
just start-all
```

### Port Already in Use

Check if another process is using port 9000:
```bash
lsof -i :9000
```

## Next Steps

- [ ] Add authentication middleware
- [ ] Implement rate limiting
- [ ] Add request validation
- [ ] Add workflow execution monitoring
- [ ] Add WebSocket support for real-time updates
