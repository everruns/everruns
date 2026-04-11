# Feature Flags

System-level feature flags that control feature availability across the platform.

## Design

### Principles

- **Env-first**: Flags are controlled via `FEATURE_<NAME>` environment variables
- **Experimental auto-enable**: Flags marked "experimental" are auto-enabled in dev (`DeploymentGrade::Dev`)
- **Explicit override wins**: `FEATURE_<NAME>=true/false` always takes priority over defaults
- **Type-safe**: `FeatureFlags` struct with named boolean fields
- **Fetch once**: UI fetches flags once on mount (React Query with 5min stale time)
- **No DB needed (yet)**: Backed by env vars + deployment grade, no database table

### Flag Resolution Order

1. Explicit env var `FEATURE_<NAME>=true|1` or `=false|0`
2. For experimental flags: `DeploymentGrade::Dev` -> `true`
3. Default: `false`

### Flag Types

- **Experimental**: Auto-enabled in dev, disabled in prod. Use for features under active development.
- **Standard**: Off by default everywhere. Enabled explicitly via env var.

### Flag Visibility

Flags have two visibility levels, modeled as separate structs:

- **`FeatureFlags`** (API-visible): Included in `GET /v1/feature-flags` JSON response. Available to frontend via `useFeatureFlag()`. Use for flags that gate UI features or user-visible behavior.
- **`InternalFeatureFlags`** (backend-only): Not serialized, not exposed via API. Used purely for internal backend gating (capability registration, infrastructure behavior). Not added to frontend types.

## Current Flags

| Flag | Type | Visibility | Env Var | Description |
|------|------|------------|---------|-------------|
| `global_chat` | Experimental | API | `FEATURE_GLOBAL_CHAT` | Per-user singleton chat session |
| `apps` | Experimental | API | `FEATURE_APPS` | Agent deployment to distribution channels |
| `notifications` | Standard | API | `FEATURE_NOTIFICATIONS` | In-app notifications (bell, toasts, SSE) |
| `mcp_endpoint` | Experimental | API | `FEATURE_MCP_ENDPOINT` | POST /mcp endpoint (Everruns as MCP server) |
| `evals` | Experimental | API | `FEATURE_EVALS` | User-facing behavioral evals for agents |
| `docker_capability` | Standard | Backend-only | `FEATURE_DOCKER_CAPABILITY` | Docker container capability (disabled by default on all envs) |

See `crates/core/src/feature_flags.rs` for the full `FeatureFlags` struct and resolution logic.

## Architecture

### Backend

- **Core**: `crates/core/src/feature_flags.rs` — `FeatureFlags` + `InternalFeatureFlags` structs, `from_env()`, resolution helpers
- **API**: `GET /v1/feature-flags` — public endpoint, returns `FeatureFlags` as JSON
- **Server**: `crates/server/src/api/feature_flags.rs` — route handler

Flags are computed once at server startup and served from memory.

### Frontend

- **API client**: `apps/ui/src/lib/api/feature-flags.ts`
- **Provider**: `apps/ui/src/providers/feature-flags-provider.tsx` — React context, fetches once
- **Hooks**: `useFeatureFlags()` (all flags), `useFeatureFlag("flag_name")` (single flag)
- **Types**: `FeatureFlags` in `apps/ui/src/lib/api/types/auth-types.ts`

### Workers

Workers currently do not need direct flag access. If needed in future, flags can be
passed via gRPC turn context.

## Adding a New Flag

### API-visible flag (gates UI or user-visible behavior)

1. Add field to `FeatureFlags` struct in `crates/core/src/feature_flags.rs`
2. Add resolution in `from_env()` using `experimental_flag()` or `standard_flag()`
3. Add to `is_enabled()` match arm
4. Update `all_enabled()`
5. Add to `FeatureFlags` interface in `apps/ui/src/lib/api/types/auth-types.ts`
6. Add to `DEFAULT_FLAGS` in `apps/ui/src/providers/feature-flags-provider.tsx`
7. Use `useFeatureFlag("flag_name")` in UI components

### Backend-only flag (gates internal behavior, not exposed to API/UI)

1. Add field to `InternalFeatureFlags` struct in `crates/core/src/feature_flags.rs`
2. Add resolution in `InternalFeatureFlags::from_env()` using `standard_flag()`
3. Add to `InternalFeatureFlags::is_enabled()` match arm
4. Do NOT add to frontend types or defaults

## UI Indication

Features gated behind experimental flags display visual badges to signal their status:

- **Sidebar**: Flask icon (`FlaskConical`) next to the nav item name, with tooltip on hover
- **Page header**: Handwritten-style "experimental" label (Caveat font) with hand-drawn circle border, placed inline next to the page title

Both are driven by `experimental: true` on the `NavItem` type in the sidebar, and `<ExperimentalPageBadge />` on individual pages. See `apps/ui/src/components/ui/experimental-badge.tsx`.

## Future Extensions

- **Per-org flags**: Add `org_id` column to a `feature_flag_overrides` table
- **Per-user flags**: Add `user_id` column
- **External providers**: Implement a `FeatureFlagProvider` trait; plug in LaunchDarkly, Unleash, etc.
- **Runtime toggle**: Admin API to update flag overrides without restart
- **Worker propagation**: Include flags in gRPC `GetTurnContext` response
