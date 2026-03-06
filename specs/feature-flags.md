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

## Current Flags

| Flag | Env Var | Type | Description |
|------|---------|------|-------------|
| `global_chat` | `FEATURE_GLOBAL_CHAT` | Experimental | Per-user singleton chat session |

## Architecture

### Backend

- **Core**: `crates/core/src/feature_flags.rs` — `FeatureFlags` struct, `from_env()`, resolution helpers
- **API**: `GET /v1/feature-flags` — public endpoint, returns `FeatureFlags` as JSON
- **Server**: `crates/server/src/api/feature_flags.rs` — route handler

Flags are computed once at server startup and served from memory.

### Frontend

- **API client**: `apps/ui/src/lib/api/feature-flags.ts`
- **Provider**: `apps/ui/src/providers/feature-flags-provider.tsx` — React context, fetches once
- **Hooks**: `useFeatureFlags()` (all flags), `useFeatureFlag("flag_name")` (single flag)
- **Types**: `FeatureFlags` in `apps/ui/src/lib/api/types.ts`

### Workers

Workers currently do not need direct flag access. If needed in future, flags can be
passed via gRPC turn context.

## Adding a New Flag

1. Add field to `FeatureFlags` struct in `crates/core/src/feature_flags.rs`
2. Add resolution in `from_env()` using `experimental_flag()` or `standard_flag()`
3. Add to `is_enabled()` match arm
4. Update `default()` and `all_enabled()`
5. Add to `FeatureFlags` interface in `apps/ui/src/lib/api/types.ts`
6. Add to `DEFAULT_FLAGS` in `apps/ui/src/providers/feature-flags-provider.tsx`
7. Use `useFeatureFlag("flag_name")` in UI components

## Future Extensions

- **Per-org flags**: Add `org_id` column to a `feature_flag_overrides` table
- **Per-user flags**: Add `user_id` column
- **External providers**: Implement a `FeatureFlagProvider` trait; plug in LaunchDarkly, Unleash, etc.
- **Runtime toggle**: Admin API to update flag overrides without restart
- **Worker propagation**: Include flags in gRPC `GetTurnContext` response
