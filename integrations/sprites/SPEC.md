# Sprites Capability Specification

## Abstract

The Sprites capability integrates [Sprites](https://sprites.dev/) (Fly.io) persistent, hardware-isolated Linux microVMs as an agent execution environment. Agents can create, manage, and interact with multiple Firecracker VMs per session via the Sprites REST API. Sprites differ from ephemeral sandboxes, they persist filesystems across idle periods, support instant checkpoint/restore, and expose public HTTP endpoints.

**Status**: Available (All environments)

## Architecture

### Single-Tier API

Sprites exposes a single unified REST API:

- **API Base**: `https://api.sprites.dev/v1`
- **Auth**: `Bearer <api_token>`
- **Operations**: Sprite lifecycle, exec, filesystem, checkpoints, services

```
┌────────────────────────────────────────────┐
│              Agent Session                  │
│                                             │
│  Tool Call (sprites_exec, etc.)             │
│         ↓                                   │
│  Load SpriteState from session KV store    │
│         ↓                                   │
│  ┌─────────────────────────────────────┐   │
│  │ SpritesClient                       │   │
│  │  - REST API (all operations)        │   │
│  └─────────────────────────────────────┘   │
│         ↓                                   │
│  Return result to agent                    │
└────────────────────────────────────────────┘
```

### State Management

Per-sprite state is stored in session **secrets** (encrypted at rest via AES-256-GCM envelope encryption):

- **Per-sprite state**: secret name `sprites_sprite:{sprite_name}`, value is JSON-serialized `SpriteState`

### API Token Resolution

The Sprites API token is resolved via **user connection** for the `sprites` provider (Settings > Connections).
If not configured, a `ConnectionRequired` result triggers the UI's inline connection dialog.

### User Connection

Sprites registers as a `ConnectionProviderPlugin` (API-key type). Users configure their token in **Settings > Connections > Sprites**.

See `src/connection.rs` for the full form schema and validation logic.

## Persistence Model

Unlike ephemeral sandbox providers (E2B, Deno), Sprites embraces persistence:

1. **Filesystem persists**: Full ext4 filesystem survives idle/hibernation, backed to durable object storage.
2. **No auto-delete**: Sprites hibernate when idle (no compute charges), but exist until explicitly deleted.
3. **Checkpoints**: Snapshot filesystem state in ~300ms for rollback capability.
4. **HTTP services**: Each sprite has a unique public URL for exposing web services on port 8080.

This means sprite names must be unique per user account (not per session). The integration tracks which sprites belong to this session via session secrets, but the sprites themselves outlive the session.

## Tool Surface

| Tool | API | Description |
|---|---|---|
| `sprites_create_sprite` | `POST /v1/sprites` | Create a new Firecracker microVM |
| `sprites_exec` | `POST /v1/sprites/{name}/exec` | Execute a shell command via `sh -lc` over the HTTP exec endpoint |
| `sprites_read_file` | `GET /v1/sprites/{name}/fs/read?path=...` | Read file from sprite filesystem |
| `sprites_write_file` | `PUT /v1/sprites/{name}/fs/write?path=...&mkdir=true` | Write file to sprite filesystem |
| `sprites_list_sprites` | (session state) | List sprites created in this session |
| `sprites_manage_sprite` | `DELETE /v1/sprites/{name}` | Delete sprite |
| `sprites_checkpoint` | `POST /v1/sprites/{name}/checkpoint` | Create filesystem checkpoint |
| `sprites_restore_checkpoint` | `POST /v1/sprites/{name}/checkpoints/{id}/restore` | Restore to checkpoint |
| `sprites_service_url` | `GET /v1/sprites/{name}` + `GET /v1/sprites/{name}/services` | Get public HTTP URL |

Default working directory inside a sprite is `/home/sprite`.

## Security Review

| Category | Assessment |
|---|---|
| **Credential storage** | API token stored in user_connections (encrypted). Never in session events. |
| **Network isolation** | Firecracker VM-level isolation (stronger than containers). L3 network policies. |
| **Data persistence** | Filesystem persists, users should be aware data outlives sessions. |
| **Leased resources** | Registered for cleanup; 30-min lease duration. |
| **Metadata labels** | Sprites tagged with `everruns.*` metadata for audit and orphan cleanup. |

## Testing

- `tests/live_api_test.rs` is feature-gated behind `sprites-live-tests`.
- Missing-credential behavior is **fail-closed**: with the feature flag on but `SPRITES_API_TOKEN` unset, the test panics. See `knowledge/integrations/integrations.md`.
- `.github/workflows/sprites-integration.yml` keeps the live job off `pull_request`, fetches `SPRITES_API_TOKEN` from Doppler before running the live tests, and runs on pushes to `main` when `integrations/sprites/**` changes. It also supports `workflow_dispatch` so credential wiring can be verified immediately after Doppler changes.
- `.github/workflows/integration-live-sweep.yml` reruns the same live path weekly and on demand so shared regressions do not hide behind that path filter. The Sprites row explicitly preflights `SPRITES_API_TOKEN` from Doppler before invoking the live test command.
