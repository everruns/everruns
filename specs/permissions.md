# Permissions Model

## Overview

Fine-grained permission system built on top of existing OrgRole hierarchy. Roles grant default permission sets through `DefaultPermissionResolver`. Policies (sets of rules) enforce access at the service layer. Policy results are exposed to UI via per-resource config endpoints. Downstream consumers can override permission resolution with `PermissionResolver` without changing policy definitions.

## Concepts

### Permission

Identifier for an action. Format: `org:<resource>:<action>`.

```
org:harnesses:manage              # CRUD on harnesses
org:harnesses:dangerous           # Delete, reset, etc.
org:agents:manage                 # CRUD on agents
org:sessions:manage               # CRUD on sessions
org:llm-providers:manage          # CRUD on LLM providers
org:settings:manage               # Org settings
org:members:manage                # Invite/remove members
org:api-keys:manage               # CRUD on API keys
org:harnesses:view                # View harnesses (read-only)
org:llm-providers:view            # View LLM providers (read-only)
org:settings:view                 # View org settings (read-only)
org:members:view                  # View org members (read-only)
```

### Rule

Single predicate that evaluates to true/false given an auth context.

Hardcoded enum variants (no dynamic rules):

| Rule | Description |
|------|-------------|
| `UserHasPermission(perm)` | User's role grants the permission |
| `UserHasRole(role)` | User has at least this OrgRole |
| `IsPlatformUser` | Caller is in platform user allowlist |
| `UserIsResourceOwner` | User created the resource (future) |

Rules are **additive within a policy** — all rules must pass (AND logic).

### Policy

Named set of rules. Attached to service operations. All rules must pass for access.

```rust
Policy {
    id: "harness.manage",
    rules: [UserHasPermission("org:harnesses:manage")],
}

Policy {
    id: "harness.dangerous",
    rules: [
        UserHasPermission("org:harnesses:manage"),
        UserHasPermission("org:harnesses:dangerous"),
    ],
}

Policy {
    id: "durable.access",
    rules: [IsPlatformUser],
}
```

## Role → Permission Mapping

Default OSS mapping is hardcoded. No DB storage (phase 1).

| Permission | Owner | Admin | Member |
|------------|-------|-------|--------|
| `org:harnesses:manage` | yes | yes | no |
| `org:harnesses:dangerous` | yes | no | no |
| `org:agents:manage` | yes | yes | yes |
| `org:sessions:manage` | yes | yes | yes |
| `org:llm-providers:manage` | yes | yes | no |
| `org:settings:manage` | yes | yes | no |
| `org:members:manage` | yes | yes | no |
| `org:api-keys:manage` | yes | yes | no |
| `org:harnesses:view` | yes | yes | yes |
| `org:llm-providers:view` | yes | yes | yes |
| `org:settings:view` | yes | yes | yes |
| `org:members:view` | yes | yes | yes |

Owner inherits all Admin permissions. Admin inherits all Member permissions.

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

The `#[policy]` macro continues to call `Policy::evaluate()`, which uses `DefaultPermissionResolver`. Custom resolvers are opt-in at explicit call sites (config endpoints, or manual `evaluate_with()` calls); the macro behavior does not change.

## Enforcement

### Service Layer

Services receive a `Caller` context (replacing raw `org_id`). Policy checks happen at the start of each service method.

```rust
/// Auth context passed from API handler to service.
pub struct Caller {
    pub org_id: i64,
    pub org_public_id: String,
    pub user_id: Option<Uuid>,
    pub role: OrgRole,
}

impl From<&ResolvedOrg> for Caller {
    fn from(org: &ResolvedOrg) -> Self { ... }
}
```

### Policy Evaluation

```rust
pub enum Rule {
    UserHasPermission(Permission),
    UserHasRole(OrgRole),
}

pub struct Policy {
    pub id: &'static str,
    pub rules: &'static [Rule],
}

impl Policy {
    /// Returns Ok(()) or Err(PolicyDenied).
    pub fn evaluate(&self, caller: &Caller) -> Result<(), PolicyError> {
        self.evaluate_with(&DefaultPermissionResolver, caller)
    }

    /// Evaluate with a custom resolver.
    pub fn evaluate_with(
        &self,
        resolver: &dyn PermissionResolver,
        caller: &Caller,
    ) -> Result<(), PolicyError> {
        for rule in self.rules {
            match rule {
                Rule::UserHasPermission(perm) => {
                    if !resolver.has_permission(caller, perm) {
                        return Err(PolicyError::denied(self.id, rule));
                    }
                }
                Rule::UserHasRole(required) => {
                    if !caller.role.has_org_role(*required) {
                        return Err(PolicyError::denied(self.id, rule));
                    }
                }
            }
        }
        Ok(())
    }
}
```

Config endpoints can use `evaluate_policies_with(resolver, caller, policies)` when they need policy results derived from a custom resolver instead of the default role map.

### Declarative Enforcement via `#[policy]` Macro

New proc macro crate: `crates/macros` (`everruns-macros`).

The `#[policy(...)]` attribute macro injects `POLICY.evaluate(caller)?;` at the top of the function body. It finds the `caller: &Caller` parameter by type automatically.

#### Usage

```rust
// Policies defined as constants
const HARNESS_MANAGE: Policy = Policy {
    id: "harness.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgHarnessesManage)],
};

const HARNESS_DANGEROUS: Policy = Policy {
    id: "harness.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgHarnessesManage),
        Rule::UserHasPermission(Permission::OrgHarnessesDangerous),
    ],
};

impl HarnessService {
    #[policy(HARNESS_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
        // just business logic — policy check injected by macro
    }

    #[policy(HARNESS_DANGEROUS)]
    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        // ...
    }
}
```

#### Macro Expansion

```rust
// #[policy(HARNESS_MANAGE)]
// pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
//     let row = self.db.create_harness(caller.org_id, input).await?;
//     Ok(harness)
// }
//
// expands to:
//
// pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
//     HARNESS_MANAGE.evaluate(caller)?;
//     let row = self.db.create_harness(caller.org_id, input).await?;
//     Ok(harness)
// }
```

#### Macro Implementation

The proc macro:

1. Parses the attribute argument as a path to a `Policy` constant (e.g. `HARNESS_MANAGE`)
2. Scans the function signature for a parameter with type `&Caller` — extracts its name (usually `caller`)
3. Prepends `#policy_path.evaluate(#caller_ident)?;` to the function body
4. Compile error if no `&Caller` parameter found

The macro continues to call `Policy::evaluate()`, which uses `DefaultPermissionResolver`. Custom resolvers are opt-in at explicit call sites; the macro behavior does not change.

```rust
// crates/macros/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn, FnArg, PatType, Type, TypeReference, Path};

#[proc_macro_attribute]
pub fn policy(attr: TokenStream, item: TokenStream) -> TokenStream {
    let policy_path = parse_macro_input!(attr as Path);
    let mut func = parse_macro_input!(item as ItemFn);

    // Find the &Caller parameter
    let caller_ident = func.sig.inputs.iter().find_map(|arg| {
        if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
            if let Type::Reference(TypeReference { elem, .. }) = ty.as_ref() {
                if let Type::Path(tp) = elem.as_ref() {
                    if tp.path.segments.last().map(|s| s.ident == "Caller").unwrap_or(false) {
                        if let syn::Pat::Ident(pi) = pat.as_ref() {
                            return Some(pi.ident.clone());
                        }
                    }
                }
            }
        }
        None
    });

    let caller = caller_ident.expect("Expected a `caller: &Caller` parameter");

    let original_body = &func.block;
    func.block = syn::parse_quote!({
        #policy_path.evaluate(#caller)?;
        #original_body
    });

    quote!(#func).into()
}
```

#### Multiple Policies

Stack multiple attributes for methods needing several independent policy checks:

```rust
#[policy(HARNESS_MANAGE)]
#[policy(SOME_OTHER_POLICY)]
pub async fn special_operation(&self, caller: &Caller) -> Result<()> {
    // both policies checked in order
}
```

#### Crate Structure

```
crates/macros/
├── Cargo.toml    # proc-macro = true, deps: syn, quote, proc-macro2
└── src/
    └── lib.rs    # #[policy] macro
```

Added to workspace `Cargo.toml` members. `crates/server` depends on `everruns-macros`.

### Error Mapping

`PolicyError` maps to HTTP 403 at the API layer.

```rust
pub struct PolicyError {
    pub policy_id: String,
    pub message: String,
}
```

API handler converts via `impl IntoResponse for PolicyError` or the existing error pipeline.

## UI: Policy Results API

Per-resource config endpoints return which policies the caller satisfies.

### Endpoint Pattern

```
GET /v1/harnesses/config
GET /v1/agents/config
GET /v1/apps/config
GET /v1/sessions/config
GET /v1/mcp-servers/config
GET /v1/llm-providers/config
GET /v1/llm-models/config
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

All new service methods MUST have `#[policy]` enforcement with a `Caller` parameter. No service method should accept raw `org_id` — use `Caller` to carry auth context. New features without policy enforcement will not pass review.

### Checklist for New Resources

1. Define view + manage policies (and dangerous if applicable)
2. Add `#[policy]` to all service methods
3. Use `Caller` parameter instead of `org_id`
4. Add `GET /v1/{resource}/config` endpoint returning `ResourceConfigResponse`
5. Wire UI with `usePolicies(resource)` hook

## Migration Path

### Phase 1 (this spec)
- Permission enum, Rule enum, Policy struct
- Caller context type
- Role → permission mapping (hardcoded)
- Service-layer enforcement
- Config endpoints for UI
- `IsPlatformUser` rule with allowlist
- No DB changes

### Phase 2 (future)
- `user_permissions` table for per-user grants beyond role defaults
- `UserIsResourceOwner` rule (needs `created_by` on resources)
- Admin UI for permission management

## Files

| File | Purpose |
|------|---------|
| `crates/core/src/permissions.rs` | Permission enum, Rule, Policy, Caller, evaluation logic |
| `crates/macros/src/lib.rs` | `#[policy]` proc macro |
| `crates/server/src/services/*.rs` | `#[policy(..)]` on service methods |
| `crates/server/src/api/*_config.rs` | Config endpoints returning policy results |
| `crates/server/src/auth/middleware.rs` | Caller extraction from ResolvedOrg |
