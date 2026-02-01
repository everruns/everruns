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
2. Retrieve the agent
3. List all agents

### Example Output

```
Creating agent...
Created agent:
   ID: agt_01234567890123456789012345
   Name: My First Agent
   Status: Active
   Created at: 2025-12-13T06:30:00Z

Retrieving agent...
Retrieved agent:
   ID: agt_01234567890123456789012345
   Name: My First Agent
   Description: A helpful AI assistant

Listing all agents...
Found 1 agent(s):
   - My First Agent (agt_01234567890123456789012345)

Example completed successfully!
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

See [specs/apis.md](../../specs/apis.md) for the complete API reference.

### Core Resources

- **Agents** - `/v1/agents` - Create, list, get, update, delete agents
- **Sessions** - `/v1/sessions` - Manage chat sessions
- **Messages** - `/v1/sessions/:id/messages` - Send/receive messages
- **Events** - `/v1/sessions/:id/events` - SSE stream for real-time updates

### System

- `GET /health` - Health check (includes version and runner mode)
- `GET /swagger-ui/` - Interactive API documentation
- `GET /api-doc/openapi.json` - OpenAPI specification

Organization context is derived from authentication (API key or session cookie).

## Development

### Build

```bash
cargo build -p everruns-server
```

### Run with Custom Port

```bash
cargo run -p everruns-server
```

### Format Code

```bash
cargo fmt -p everruns-server
```

### Lint

```bash
cargo clippy -p everruns-server -- -D warnings
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

Default: `postgres://everruns:everruns@localhost:5432/everruns`

## Testing with cURL

### Create an Agent

```bash
curl -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Agent",
    "system_prompt": "You are a helpful assistant."
  }' | jq
```

### List Agents

```bash
curl http://localhost:9000/v1/agents | jq
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

## Completed Features (formerly "Next Steps")

- [x] Add authentication middleware (`src/auth/middleware.rs`)
- [x] Implement rate limiting (via circuit breakers in `everruns-durable`)
- [x] Add request validation (Axum extractors with serde validation)
- [x] Add workflow execution monitoring (durable execution engine)
- [x] Add real-time updates via SSE (`src/api/sse.rs`, `src/api/events.rs`)
