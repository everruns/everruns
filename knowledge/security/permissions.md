---
type: Specification
title: "Permissions Model"
description: "Fine-grained permissions model."
tags:
  - everruns
  - security
---
# Permissions Model

## Overview

Fine-grained permission system built on top of existing OrgRole hierarchy. Roles grant default permission sets through `DefaultPermissionResolver`. Policies (sets of rules) enforce access at the service layer. Policy results are exposed to UI via per-resource config endpoints. Downstream consumers can override permission resolution with `PermissionResolver` without changing policy definitions.

## Concepts

### Permission

Identifier for an action. Format: `org:<resource>:<action>`.

See `crates/core/src/permissions.rs` for the full `Permission` enum with all variants.

Permissions are scoped **per domain**: each managed resource (apps, MCP servers, plugins, skills, capabilities, agent identities, harnesses, providers, …) has its own `Org<Domain>View` / `Org<Domain>Manage` (and, where applicable, `Org<Domain>Dangerous`) permission rather than sharing one coarse grant. This keeps the role map least-privilege-ready: a custom resolver or a future role can grant one domain without implying the others (EVE-656, TM-AUTHZ-013).

### Rule

Single predicate that evaluates to true/false given an auth context. Hardcoded enum variants (no dynamic rules). Rules are **additive within a policy**: all rules must pass (AND logic).

See `crates/core/src/permissions.rs` for the `Rule` enum variants and `Policy` struct definitions.

## Role → Permission Mapping

Default OSS mapping is hardcoded in `DefaultPermissionResolver`. No DB storage (phase 1). Owner inherits all Admin permissions. Admin inherits all Member permissions. See `crates/core/src/permissions.rs` for the full role-to-permission mapping.

**Default role:** `Owner` (phase 1). All users get full permissions by default. Future phases will assign roles via invitation/admin UI.

## Permission Resolver Contract

`PermissionResolver` is the extensibility contract between policy evaluation and the source of truth for caller permissions.

- `DefaultPermissionResolver` preserves current OSS behavior by delegating to the hardcoded role map in `crates/core/src/permissions.rs`.
- Downstream consumers can provide a custom resolver to enforce subscription limits, per-user grants, or external RBAC decisions while reusing the same `Policy` and `Rule` types.
- Implementations must fail closed: `has_permission()` returns `true` only for real grants.
- Implementations must stay consistent: if `has_permission(caller, p)` is `true`, then `caller_permissions(caller)` must include `p`.
- Implementations must be `Send + Sync` so request handlers can share them safely.

Example custom resolvers:

- Billing-tier aware resolver that removes dangerous or premium permissions for lower plans
- Resolver backed by a per-user grants table
- Adapter that maps external RBAC roles into OSS `Permission` values

### Server Integration

`AuthState` carries the active `PermissionResolver` as `Arc<dyn PermissionResolver>`. All config endpoints (`GET /v1/{resource}/config`) use `evaluate_policies_with(auth.permission_resolver.as_ref(), ...)` so they automatically respect the injected resolver.

```rust
// OSS default (uses DefaultPermissionResolver)
AuthState::new(config, backend)

// Custom resolver
AuthState::with_resolver(config, backend, Arc::new(MyResolver))
```

Enforcement uses the resolver threaded through `Ctx`, every `Command::run` calls `policy.evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)`, so SaaS-custom resolvers apply uniformly across HTTP/MCP/gRPC/platform callers. Config endpoints (`/v1/{resource}/config`) use `evaluate_policies_with(resolver, caller, policies)` for UI gating hints.

### Platform User Contract

OSS exposes platform/system authorization as explicit auth context, not tenant-role inference:

- `AuthUser.is_platform_user` is the first-class flag returned by the auth backend.
- `ResolvedOrg` preserves that flag when HTTP auth is converted into a `Caller`.
- `Rule::IsPlatformUser` is the gate for global/system surfaces such as `/v1/durable/*`.
- `GET /v1/durable/config` exposes durable policy results so wrapper UIs can gate navigation from the same contract.

Built-in OSS auth preserves previous behavior by deriving `is_platform_user` from the `admin` auth role. Wrappers can supply their own value from their auth backend without mapping tenant org roles into global platform access.

## Enforcement

### Command Runner (Primary Enforcement Point)

Every user-facing operation is a domain `Command` (`crates/server/src/domains/common.rs`). Each command declares its policy statically via `Command::policy()`. All four caller kinds, HTTP adapters, MCP dispatch, gRPC `ExecuteCommand`, and platform capability, invoke commands through **`Command::run`**, which evaluates `policy()` against the `PermissionResolver` carried in `Ctx` before delegating to `execute()`:

```rust
impl Command for CreateAgent {
    fn policy() -> Option<&'static Policy> { Some(&AGENT_MANAGE) }
    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> { ... }
}
```

**Key properties:**

- `Command::run` is the single public entry point. `execute` is the business-logic hook and must not be called directly from HTTP/MCP/gRPC adapters, doing so skips policy evaluation. A command may call another command's `execute` as an already-authorized composition.
- `dispatch()` (used by MCP and gRPC `ExecuteCommand`) routes through `run`, enforcement is identical across HTTP and RPC paths.
- The resolver is threaded through `Ctx::new` from `AuthState.permission_resolver`. Internal callers (`Caller::internal`) use `DefaultPermissionResolver` so SaaS-custom restrictions never block internal ops.

**Inventory coverage test.** `crates/server/tests/command_policy_enforcement_test.rs` iterates `inventory::iter::<CommandDescriptor>` and asserts every non-GET command declares a policy. New mutating commands that forget `policy()` fail the build.

### Retired: `#[policy]` Macro

The `#[policy(...)]` attribute macro has been removed. It was the historical enforcement mechanism (inject `POLICY.evaluate(caller)?;` at the top of a service method) and hardcoded `DefaultPermissionResolver`, which bypassed the SaaS resolver contract (TM-AUTHZ-008). `Command::run` is now the single enforcement point and uses `policy.evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)`. Service methods no longer perform policy checks; the authorization check happens at the command boundary.

### Durable Ownership

Policy evaluation and durable ownership are separate concerns.

- `Caller.user_id` is request-time auth context.
- Durable resources use `owner_principal_id` to capture who owns the resource over time.
- Ownership resolution is principal-based, not direct-user-based: `agent_identity` and `system` principals must resolve through lineage to an effective human owner when the resource is user-scoped.
- Ownership transfer is entity-scoped. Updating a resident `agent_identity_id` must not silently transfer the effective human owner.

### Policy Evaluation

See `crates/core/src/permissions.rs` for evaluation logic. Policies support both default and custom resolvers via `evaluate()` / `evaluate_with()`. The command runner always uses `policy.evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)` so custom resolvers (SaaS billing-tier, per-user grants, external RBAC) apply uniformly across HTTP/MCP/gRPC/platform. Config endpoints use `evaluate_policies_with(resolver, caller, policies)` for UI-facing policy hints.

### Error Mapping

`PolicyError` maps to HTTP 403 at the API layer. See `crates/core/src/permissions.rs` for the `PolicyError` type.

### Capability Risk Gate

Besides the `Permission` / `Policy` contract there is one hardcoded role gate in agent create/update enforcement: **assigning a `RiskLevel::High` capability to an agent requires `OrgRole::Admin`**.

- **Where.** Canonical create/update enforcement is `check_high_risk_caps` in `crates/server/src/domains/agents/commands.rs` (invoked from `CreateAgent::execute`, `UpdateAgent::execute`, and `UpsertAgent::execute`). The sibling `require_admin_for_high_risk` helper in `crates/server/src/api/agents.rs` enforces the same contract on agent-import / copy paths (TM-AGENT-005).
- **Trigger.** Any capability whose `risk_level()` returns `High`, the canonical, current set is enumerated in `knowledge/execution/capabilities.md` and discoverable via `rg "RiskLevel::High" crates/core integrations` (sandbox/exec capabilities such as `docker_container`, `daytona`, `e2b`, `deno`, `bashkit_shell` plus others including `web_fetch`, `user_hooks`, and `lua`); it is not frozen here. The full contract, including the rationale for `bashkit_shell` / `web_fetch` and migration semantics for grandfathered agents, lives in `knowledge/execution/capabilities.md` ("Admin-Only Tier Decision").
- **Failure mode.** HTTP 403 with the offending capability ids; the request does not partially succeed.
- **Why it lives outside the `Permission` enum.** The gate is based on per-capability metadata, not a per-action permission, and the set of capabilities is open (extensions can add them). Keeping it as a centralized hardcoded check in the agent enforcement path avoids a combinatorial explosion of `org:capability:<id>` permissions while preserving an explicit trust boundary.

## UI: Policy Results API

Per-resource config endpoints return which policies the caller satisfies.

### Endpoint Pattern

```
GET /v1/harnesses/config
GET /v1/agents/config
GET /v1/apps/config
GET /v1/sessions/config
GET /v1/mcp-servers/config
GET /v1/providers/config
GET /v1/models/config
GET /v1/skills/config
```

### Response

```json
{
  "policies": {
    "harness.manage": true,
    "harness.dangerous": false
  }
}
```

UI uses this on load to show/hide controls (delete buttons, admin panels, etc.).

### Implementation

Each resource module defines its relevant policies. The config handler evaluates all of them against the caller:

```rust
pub async fn harness_config(org: ResolvedOrg) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    Json(ResourceConfigResponse {
        policies: evaluate_policies(&caller, &[HARNESS_MANAGE, HARNESS_DANGEROUS]),
    })
}
```

## Policy Requirements for New Features

Every non-GET domain command MUST declare `Command::policy() -> Option<&'static Policy>`. The command runner enforces the declared policy via `policy.evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)` before `execute`. `crates/server/tests/command_policy_enforcement_test.rs::every_non_readonly_command_declares_policy` walks `inventory::iter::<CommandDescriptor>` at test time and fails the build for any mutating command without a policy declaration.

### Checklist for New Resources

1. Define view + manage policies (and dangerous if applicable) as `pub const` in the domain's `mod.rs`.
2. Declare `fn policy() -> Option<&'static Policy>` on every `Command` impl that isn't a plain GET read.
3. Thread the resolver through `AppState.ctx()`: pass `self.auth.permission_resolver.clone()` to `Ctx::new` / `Ctx::minimal`.
4. Add `GET /v1/{resource}/config` endpoint returning `ResourceConfigResponse` for UI gating.
5. Wire UI with `usePolicies(resource)` hook.

## Migration Path

### Phase 1 (this spec)
- Permission enum, Rule enum, Policy struct
- Caller context type
- Role → permission mapping (hardcoded)
- Service-layer enforcement
- Config endpoints for UI
- `IsPlatformUser` rule with allowlist
- `AuthUser.is_platform_user` and `ResolvedOrg.is_platform_user` carry wrapper-defined platform access through HTTP caller construction
- `/v1/durable/config` exposes platform-user policy results for UI gating
- No DB changes

### Phase 2 (future)
- `user_permissions` table for per-user grants beyond role defaults
- `UserIsResourceOwner` rule (needs `created_by` on resources)
- Admin UI for permission management

## Files

| File | Purpose |
|------|---------|
| `crates/core/src/permissions.rs` | Permission enum, Rule, Policy, Caller, evaluation logic |
| `crates/server/src/domains/common.rs` | `Command::run`, `Ctx.permission_resolver`, dispatch |
| `crates/server/src/domains/*/mod.rs` | Per-domain `Policy` constants |
| `crates/server/src/domains/*/commands.rs` | `Command::policy()` declarations |
| `crates/server/tests/command_policy_enforcement_test.rs` | Inventory coverage test + resolver enforcement tests |
| `crates/server/src/auth/middleware.rs` | `Caller` extraction from `ResolvedOrg`, `AuthState.permission_resolver` |
