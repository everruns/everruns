---
type: Specification
title: "Feature Flags"
description: "Feature flags system."
tags:
  - everruns
  - security
---
# Feature Flags

System-level feature flags that control feature availability across the platform.

## Design

### Principles

- **Env-first**: Flags are controlled via `FEATURE_<NAME>` environment variables
- **Experimental auto-enable**: Flags marked "experimental" are auto-enabled in dev (`DeploymentGrade::Dev`)
- **Explicit override wins**: `FEATURE_<NAME>=true/false` always takes priority over defaults
- **Type-safe**: `FeatureFlags` struct with named boolean fields
- **Fetch once**: UI fetches flags once on mount (React Query with 5min stale time)
- **Org opt-in**: Organizations enable API-visible flags when the deployment allows them (stored in `org_feature_flags`)

### Flag Resolution Order

**Deployment (system) flags:**

1. Explicit env var `FEATURE_<NAME>=true|1` or `=false|0`
2. For experimental flags: `DeploymentGrade::Dev` -> `true`
3. Default: `false`

**Effective flags for an organization:**

1. System flag must be enabled on the deployment
2. Organization must have opted in (`org_feature_flags.enabled = true`; default is off)
3. `effective = system_enabled && org_enabled`

### Flag Types

- **Experimental**: Auto-enabled in dev, disabled in prod. Use for features under active development.
- **Standard**: Off by default everywhere. Enabled explicitly via env var.

### Flag Visibility

Flags have two visibility levels, modeled as separate structs:

- **`FeatureFlags`** (API-visible): Included in `GET /v1/feature-flags` JSON response. Available to frontend via `useFeatureFlag()`. Use for flags that gate UI features or user-visible behavior.
- **`InternalFeatureFlags`** (backend-only): Not serialized, not exposed via API. Used purely for internal backend gating (capability registration, infrastructure behavior). Not added to frontend types.

## Current Flags

See `crates/platform/src/feature_flags.rs` for the complete list of flags and their resolution logic.

Planned flag:

| Flag | Type | Visibility | Env var | Purpose |
|------|------|------------|---------|---------|
| `voice` | Experimental | API-visible | `FEATURE_VOICE` | Enables Realtime voice endpoints and microphone controls in session chat, agent chat, and Platform Chat. |

Current API-visible experimental flags include:

- `skills`, `memory`, `knowledge`, and `plugins`: gate their management pages, sidebar entries,
  global-search results, management APIs, Platform/MCP command discovery, and corresponding runtime
  capabilities. Direct calls return `feature_not_enabled` while the org-effective flag is off.
- `agent_versions`: gates immutable Agent snapshots, forks, rollback, version diffs, and App version binding. See `knowledge/runtime-resources/agent-versions.md`.
- `agent_delegation`: gates outbound agent delegation capabilities (`a2a_agent_delegation`, `agent_handoff`). Deployment disablement prevents registration; org-effective disablement removes them from API and Platform listings, assignment, and runtime tool construction. Env var: `FEATURE_AGENT_DELEGATION`. See EVE-506.
- `observers`: gates online scoring of production sessions (`/v1/observers`), the `turn.completed` matching listener, and the background scoring worker. When off, no observer routes are mounted and no listener/worker is registered. Env var: `FEATURE_OBSERVERS`. See `knowledge/evaluation/online-evals.md`.
- `public_chat`: gates the Public Chat feature — the public endpoints (`/v1/apps/{app_id}/public-chat[/config]`), `public_chat` channel creation/editing, the builder UI (channel-type picker), and the public web route. The public endpoints are gated on the deployment flag; channel creation and the builder UI are gated on the org-effective flag. Env var: `FEATURE_PUBLIC_CHAT`. See `knowledge/integrations/public-chat.md`.
- `webmcp`: gates browser-native tools exposed by the authenticated UI. The deployment gate also controls the `tools` Permissions Policy; org opt-in controls registration. Env var: `FEATURE_WEBMCP`. See `knowledge/ui/webmcp.md`.

## Architecture

### Backend

- **Platform**: `crates/platform/src/feature_flags.rs` — `FeatureFlags` records, catalog, org opt-in resolution, `from_env()` (EVE-878)
- **Core**: `crates/core/src/execution_features.rs` — `InternalFeatureFlags` and the resolved `ExecutionFeatureDecisions` consumed at capability registration; execution never loads feature-management records
- **API**: `GET /v1/feature-flags` — public endpoint, returns deployment-level `FeatureFlags` as JSON
- **Org API**: `GET/PATCH /v1/orgs/{org}/feature-flags`, `GET /v1/orgs/{org}/feature-flags/settings` — org opt-in (admin for PATCH)
- **Server**: `crates/server/src/api/feature_flags.rs`, `crates/server/src/api/org_feature_flags.rs`
- **Storage**: `org_feature_flags` table (migration `046_org_feature_flags.sql`)

Deployment flags are computed once at server startup. Org-effective flags are resolved per request from system flags + `org_feature_flags` rows (`ResolvedOrg::feature_flags`, `Ctx::feature_flags`).
Domain command metadata maps feature-owned command categories to those effective flags, keeping HTTP,
Platform, and MCP execution/discovery aligned. Capability listing, assignment, and turn-context
assembly apply the same effective flags so persisted configurations cannot re-expose a disabled
capability.

### Frontend

- **API client**: `apps/ui/src/lib/api/feature-flags.ts`
- **Provider**: `apps/ui/src/providers/feature-flags-provider.tsx` — fetches org-effective flags when an org is selected
- **Settings**: `apps/ui/src/app/(main)/settings/features/page.tsx` — admin opt-in toggles
- **Hooks**: `useFeatureFlags()` (effective flags), `useFeatureFlag("flag_name")` (single flag)
- **Types**: `FeatureFlags` in `apps/ui/src/lib/api/legacy-api-types.ts`

### Workers

Server/worker boundaries resolve the org-effective flags before Platform command dispatch and turn
context assembly. Disabled feature-owned capabilities are removed before runtime tool construction.

## Adding a New Flag

### API-visible flag (gates UI or user-visible behavior)

1. Add field to `FeatureFlags` struct in `crates/platform/src/feature_flags.rs`
2. Add resolution in `from_env()` using `experimental_flag()` or `standard_flag()`
3. Add to `is_enabled()` match arm
4. Update `all_enabled()`
5. Add to `FeatureFlags` interface in `apps/ui/src/lib/api/legacy-api-types.ts`
6. Add to `DEFAULT_FLAGS` in `apps/ui/src/providers/feature-flags-provider.tsx`
7. Use `useFeatureFlag("flag_name")` in UI components
8. If the feature owns API commands or capabilities, add its command-category/capability mapping to
   `crates/server/src/domains/common.rs` and `crates/platform/src/feature_flags.rs`

### Backend-only flag (gates internal behavior, not exposed to API/UI)

1. Add field to `InternalFeatureFlags` struct in `crates/core/src/execution_features.rs`
2. Add resolution in `InternalFeatureFlags::from_env()` using `standard_flag()`
3. Add to `InternalFeatureFlags::is_enabled()` match arm
4. Do NOT add to frontend types or defaults

## UI Indication

Features gated behind experimental flags display a visual marker to signal their status:

- **Sidebar**: Flask icon (`FlaskConical`) next to the nav item name, with tooltip on hover

The marker is driven by `experimental: true` on the `NavItem` type in the sidebar. See
`apps/ui/src/components/ui/experimental-badge.tsx`.

## Future Extensions

- **Per-user flags**: Add `user_id` column for user-level overrides within an org
- **External providers**: Implement a `FeatureFlagProvider` trait; plug in LaunchDarkly, Unleash, etc.
- **Runtime toggle**: Admin API to update flag overrides without restart
- **Worker propagation**: Include flags in gRPC `GetTurnContext` response
