# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable AI agent execution platform.

## Requirements

### Agent
Represents an AI assistant configuration.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Display name |
| `description` | string? | Optional description |
| `default_model_id` | string | Default LLM model identifier |
| `definition` | JSON | Agent configuration (system prompt, LLM settings) |
| `status` | enum | `active` or `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

Definition schema:
```json
{
  "system": "System prompt text",
  "llm": {
    "temperature": 0.7,
    "max_tokens": 1000
  }
}
```

### Thread
Represents a conversation context.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `created_at` | timestamp | Creation time |

### Message
A single message in a thread.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `thread_id` | UUID v7 | Parent thread reference |
| `role` | enum | `system`, `user`, `assistant`, `tool` |
| `content` | string | Message content |
| `created_at` | timestamp | Creation time |

### Run
A single execution of an agent on a thread.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `agent_id` | UUID v7 | Agent reference |
| `thread_id` | UUID v7 | Thread reference |
| `status` | enum | `pending`, `running`, `completed`, `failed`, `cancelled` |
| `created_at` | timestamp | Creation time |
| `started_at` | timestamp? | Execution start time |
| `finished_at` | timestamp? | Execution end time |

Status transitions: `pending` → `running` → `completed` | `failed` | `cancelled`

### RunEvent
AG-UI event emitted during run execution.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `run_id` | UUID v7 | Parent run reference |
| `sequence` | integer | Order within run |
| `event_type` | string | AG-UI event type |
| `payload` | JSON | Event data |
| `created_at` | timestamp | Creation time |
