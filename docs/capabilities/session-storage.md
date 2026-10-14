---
title: Session Storage
description: Session-scoped key/value storage and encrypted secret storage.
sidebar:
  label: Storage
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `session_storage` |
| **Category** | Storage |
| **Features** | `secrets`, `key_value` (enables Storage tab) |
| **Dependencies** | None |

Two storage mechanisms scoped to the current session:

- **Key/Value store**: plain-text storage for general data
- **Secret store**: AES-256-GCM encrypted storage for sensitive data

## Tools

### `kv_store`

Manage plain-text key/value pairs.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `set`, `get`, `delete`, or `list` |
| `key` | string | conditional | Required for `set`, `get`, `delete` |
| `value` | string | conditional | Required for `set` |

### `secret_store`

Manage encrypted secrets. Values are encrypted at rest and write-only: an agent can store, list and delete secrets, but no tool reads a value back.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `set`, `delete`, or `list` |
| `name` | string | conditional | Required for `set` and `delete` |
| `value` | string | conditional | Required for `set` |

## Notes

- Data is session-scoped, no cross-session access
- `set` uses upsert semantics (overwrites existing keys)
- Secret operations require `SECRETS_ENCRYPTION_KEY` to be configured
- `list` returns keys/names only (not values) for secrets
- The Storage tab can create, replace, and delete values, but never reads a value back
- A session secret is available only to that session. It does not follow an Agent Trigger that creates a session per invocation.
- No secret value ever reaches the model. `secret_store get` answers the same refusal for every name. Capabilities that need a key (GitHub, Brave Search, sandboxes and others) read their own secret on the server. A key the agent's shell sends to an HTTP API is a bound secret, described next; an MCP tool-parameter credential goes on the Agent's **Credentials** tab.
- A **bound secret** (stored with a URL to send it to) is write-only: `secret_store list` shows its name, URL and how to use it, while `get`, `set` and `delete` refuse it. The Bashkit shell uses it as `$NAME` in HTTP requests to that URL; see [Bashkit Shell](/capabilities/bashkit-shell/#api-keys-the-agent-never-sees). To give every session and trigger of an agent the same key, add it as an **HTTP API** credential on the Agent's **Credentials** tab.

## See Also

- [Session](/capabilities/session/), session metadata
- [File System](/capabilities/file-system/), file-based storage alternative
- [Capabilities Overview](/capabilities/)
