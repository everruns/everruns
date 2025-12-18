# Using Everruns via APIs

This tutorial walks you through getting API keys and making API calls to interact with Everruns programmatically.

## Overview

Everruns provides a REST API that allows you to:
- Create and manage AI agents
- Run conversations through sessions
- Send messages and receive responses
- Stream real-time events via SSE
- Configure LLM providers and models

**Base URL:** `http://localhost:9000` (local development)

**API Documentation:** Available at `/swagger-ui/` for interactive exploration.

## Prerequisites

Before using the API, ensure Everruns is running:

```bash
# Start all services
./scripts/dev.sh start-all
```

Verify the API is available:

```bash
curl http://localhost:9000/health
```

Expected response:
```json
{"status":"ok","version":"0.1.0","runner_mode":"InProcess","auth_mode":"None"}
```

## Getting API Keys

How you obtain API keys depends on the authentication mode configured for your Everruns instance.

### Step 1: Check Authentication Mode

```bash
curl http://localhost:9000/v1/auth/config
```

Example response:
```json
{
  "mode": "admin",
  "password_auth_enabled": true,
  "oauth_providers": [],
  "signup_enabled": false
}
```

The `mode` field indicates which authentication mode is active:
- `none` - No authentication required (skip to [Making API Calls](#making-api-calls))
- `admin` - Single admin user authentication
- `full` - Complete authentication with user accounts and API keys

### Step 2: Authenticate (Admin or Full Mode)

#### Option A: Login with Email/Password

```bash
# Login to get access token
curl -X POST http://localhost:9000/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "email": "admin@example.com",
    "password": "your-password"
  }'
```

Response:
```json
{
  "access_token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "refresh_token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

Save the `access_token` for API requests. It expires after 15 minutes by default.

#### Option B: OAuth (Full Mode Only)

If OAuth is configured, redirect users to:
- Google: `GET /v1/auth/oauth/google`
- GitHub: `GET /v1/auth/oauth/github`

### Step 3: Create an API Key (Full Mode)

API keys are long-lived credentials ideal for scripts and integrations.

```bash
# Store your access token
TOKEN="your-access-token-here"

# Create an API key
curl -X POST http://localhost:9000/v1/auth/api-keys \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-api-key",
    "scopes": ["agents:read", "agents:write", "sessions:read", "sessions:write"]
  }'
```

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "my-api-key",
  "key": "evr_1234567890abcdef...",
  "scopes": ["agents:read", "agents:write", "sessions:read", "sessions:write"],
  "created_at": "2024-01-15T10:30:00Z"
}
```

**Important:** The full API key (`evr_...`) is shown only once. Save it securely.

### Managing API Keys

```bash
# List your API keys
curl -X GET http://localhost:9000/v1/auth/api-keys \
  -H "Authorization: Bearer $TOKEN"

# Delete an API key
curl -X DELETE http://localhost:9000/v1/auth/api-keys/{key_id} \
  -H "Authorization: Bearer $TOKEN"
```

## Making API Calls

### Authentication Header

Include your credentials in the `Authorization` header:

```bash
# Using API key
Authorization: evr_your-api-key-here

# Using JWT token
Authorization: Bearer your-jwt-token-here
```

### Content Type

All POST/PATCH requests require JSON:

```bash
Content-Type: application/json
```

## API Endpoints

### Health Check

```bash
curl http://localhost:9000/health
```

### Agents

Agents are AI assistants defined by a system prompt and configuration.

#### Create an Agent

```bash
curl -X POST http://localhost:9000/v1/agents \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Customer Support Agent",
    "description": "Handles customer inquiries",
    "system_prompt": "You are a helpful customer support agent. Be friendly, concise, and solve problems efficiently.",
    "tags": ["support", "customer-facing"]
  }'
```

Response (201 Created):
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Customer Support Agent",
  "description": "Handles customer inquiries",
  "system_prompt": "You are a helpful customer support agent...",
  "tags": ["support", "customer-facing"],
  "status": "active",
  "created_at": "2024-01-15T10:30:00Z",
  "updated_at": "2024-01-15T10:30:00Z"
}
```

#### List Agents

```bash
curl http://localhost:9000/v1/agents \
  -H "Authorization: evr_your-api-key"
```

Response:
```json
{
  "data": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Customer Support Agent",
      "status": "active",
      ...
    }
  ]
}
```

#### Get Agent by ID

```bash
curl http://localhost:9000/v1/agents/{agent_id} \
  -H "Authorization: evr_your-api-key"
```

#### Update Agent

```bash
curl -X PATCH http://localhost:9000/v1/agents/{agent_id} \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Updated Agent Name",
    "system_prompt": "Updated system prompt..."
  }'
```

#### Archive Agent

```bash
curl -X DELETE http://localhost:9000/v1/agents/{agent_id} \
  -H "Authorization: evr_your-api-key"
```

### Sessions

Sessions represent conversation instances with an agent.

#### Create a Session

```bash
curl -X POST http://localhost:9000/v1/agents/{agent_id}/sessions \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Support Chat #1234"
  }'
```

Response (201 Created):
```json
{
  "id": "660e8400-e29b-41d4-a716-446655440001",
  "agent_id": "550e8400-e29b-41d4-a716-446655440000",
  "title": "Support Chat #1234",
  "status": "pending",
  "created_at": "2024-01-15T10:35:00Z"
}
```

#### List Sessions

```bash
curl http://localhost:9000/v1/agents/{agent_id}/sessions \
  -H "Authorization: evr_your-api-key"
```

#### Get Session

```bash
curl http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id} \
  -H "Authorization: evr_your-api-key"
```

### Messages

Messages store conversation content. Creating a user message triggers the AI workflow.

#### Send a User Message

```bash
curl -X POST http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/messages \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "role": "user",
    "content": {
      "text": "How do I reset my password?"
    }
  }'
```

Response (201 Created):
```json
{
  "id": "770e8400-e29b-41d4-a716-446655440002",
  "session_id": "660e8400-e29b-41d4-a716-446655440001",
  "sequence": 1,
  "role": "user",
  "content": {
    "text": "How do I reset my password?"
  },
  "created_at": "2024-01-15T10:36:00Z"
}
```

This automatically starts the AI workflow. The session status transitions: `pending` → `running` → `pending`.

#### List Messages

```bash
curl http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/messages \
  -H "Authorization: evr_your-api-key"
```

Response:
```json
{
  "data": [
    {
      "id": "770e8400...",
      "sequence": 1,
      "role": "user",
      "content": {"text": "How do I reset my password?"}
    },
    {
      "id": "880e8400...",
      "sequence": 2,
      "role": "assistant",
      "content": {"text": "To reset your password, follow these steps..."}
    }
  ]
}
```

### Events (Server-Sent Events)

Stream real-time updates for a session using SSE.

```bash
curl -N http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/events \
  -H "Authorization: evr_your-api-key" \
  -H "Accept: text/event-stream"
```

Event types:
- `session.started` - Session began processing
- `step.started` - LLM step started
- `message.delta` - Streaming content chunk
- `message.created` - Complete message available
- `tool.started` / `tool.completed` - Tool execution
- `session.completed` / `session.failed` - Session finished

Example event stream:
```
event: session.started
data: {"session_id": "660e8400..."}

event: step.started
data: {"step_id": "abc123"}

event: message.delta
data: {"delta": "To reset"}

event: message.delta
data: {"delta": " your password"}

event: message.created
data: {"message_id": "880e8400..."}

event: session.completed
data: {"session_id": "660e8400..."}
```

### LLM Providers

Configure AI model providers (OpenAI, Anthropic, etc.).

#### Create Provider

```bash
curl -X POST http://localhost:9000/v1/llm-providers \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI",
    "provider_type": "openai",
    "api_key": "sk-..."
  }'
```

#### List Providers

```bash
curl http://localhost:9000/v1/llm-providers \
  -H "Authorization: evr_your-api-key"
```

### LLM Models

Configure specific models for providers.

#### Create Model

```bash
curl -X POST http://localhost:9000/v1/llm-providers/{provider_id}/models \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "gpt-4",
    "display_name": "GPT-4",
    "context_window": 128000
  }'
```

#### List All Models

```bash
curl http://localhost:9000/v1/llm-models \
  -H "Authorization: evr_your-api-key"
```

### Capabilities

Enable modular functionality for agents.

#### List Available Capabilities

```bash
curl http://localhost:9000/v1/capabilities \
  -H "Authorization: evr_your-api-key"
```

#### Get Agent Capabilities

```bash
curl http://localhost:9000/v1/agents/{agent_id}/capabilities \
  -H "Authorization: evr_your-api-key"
```

#### Set Agent Capabilities

```bash
curl -X PUT http://localhost:9000/v1/agents/{agent_id}/capabilities \
  -H "Authorization: evr_your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "capability_ids": ["current_time", "noop"]
  }'
```

## Complete Example: Chat with an Agent

Here's a full workflow using shell variables:

```bash
#!/bin/bash

BASE_URL="http://localhost:9000"
API_KEY="evr_your-api-key"

# 1. Create an agent
AGENT=$(curl -s -X POST "$BASE_URL/v1/agents" \
  -H "Authorization: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Assistant",
    "system_prompt": "You are a helpful assistant."
  }')

AGENT_ID=$(echo $AGENT | jq -r '.id')
echo "Created agent: $AGENT_ID"

# 2. Create a session
SESSION=$(curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions" \
  -H "Authorization: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"title": "Test Chat"}')

SESSION_ID=$(echo $SESSION | jq -r '.id')
echo "Created session: $SESSION_ID"

# 3. Send a message
MESSAGE=$(curl -s -X POST "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Authorization: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "role": "user",
    "content": {"text": "Hello! What can you help me with?"}
  }')

echo "Sent message: $(echo $MESSAGE | jq -r '.id')"

# 4. Wait for processing and get messages
sleep 2

MESSAGES=$(curl -s "$BASE_URL/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Authorization: $API_KEY")

echo "Conversation:"
echo $MESSAGES | jq '.data[] | "\(.role): \(.content.text)"'
```

## Error Handling

The API returns standard HTTP status codes:

| Status | Meaning |
|--------|---------|
| 200 | Success |
| 201 | Created |
| 204 | No Content (successful delete) |
| 400 | Bad Request - Invalid input |
| 401 | Unauthorized - Missing or invalid credentials |
| 403 | Forbidden - Insufficient permissions |
| 404 | Not Found - Resource doesn't exist |
| 500 | Internal Server Error |

Error response format:
```json
{
  "error": "Description of the error",
  "status": 400
}
```

## Token Refresh

Access tokens expire after 15 minutes. Use the refresh token to get a new access token:

```bash
curl -X POST http://localhost:9000/v1/auth/refresh \
  -H "Content-Type: application/json" \
  -d '{
    "refresh_token": "your-refresh-token"
  }'
```

## Next Steps

- Explore the interactive API docs at `/swagger-ui/`
- Download the OpenAPI spec at `/api-doc/openapi.json`
- Check out the [Authentication Runbook](../sre/runbooks/authentication.md) for advanced configuration
- Review [Environment Variables](../sre/environment-variables.md) for deployment options
