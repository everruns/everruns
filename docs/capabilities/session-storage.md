---
title: Session Storage for Agent State and Secrets
description: Persist agent state with session-scoped key/value storage and encrypted secret storage for multi-turn workflows.
sidebar:
  label: Storage
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

Manage encrypted secrets. Same interface as `kv_store` but values are encrypted at rest.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `set`, `get`, `delete`, or `list` |
| `name` | string | conditional | Required for `set`, `get`, `delete` |
| `value` | string | conditional | Required for `set` |

## Notes

- Data is session-scoped, no cross-session access
- `set` uses upsert semantics (overwrites existing keys)
- Secret operations require `SECRETS_ENCRYPTION_KEY` to be configured
- `list` returns keys/names only (not values) for secrets
- The Storage tab can create, replace, and delete values, but never reads a value back
- A session secret is available only to that session. It does not follow an Agent Trigger that creates a session per invocation.
- `secret_store get` exposes the decrypted value to the running model. Do not use session secrets for MCP tool-parameter credentials that must stay out of model context; configure those on the Agent's **Credentials** tab instead.

## See Also

- [Session](/capabilities/session/), session metadata
- [File System](/capabilities/file-system/), file-based storage alternative
- [Capabilities Overview](/capabilities/)
