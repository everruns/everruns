# Permissions Model

## Overview

Fine-grained permission system built on top of existing OrgRole hierarchy. Roles grant default permission sets. Policies (sets of rules) enforce access at the service layer. Policy results are exposed to UI via per-resource config endpoints.

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
```

### Rule

Single predicate that evaluates to true/false given an auth context.

Hardcoded enum variants (no dynamic rules):

| Rule | Description |
|------|-------------|
| `UserHasPermission(perm)` | User's role grants the permission |
| `UserHasRole(role)` | User has at least this OrgRole |
| `UserIsResourceOwner` | User created the resource (future) |
| `IsPlatformUser` | User is in platform allowlist (future) |

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

Hardcoded. No DB storage (phase 1).

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

Owner inherits all Admin permissions. Admin inherits all Member permissions.

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
        for rule in self.rules {
            match rule {
                Rule::UserHasPermission(perm) => {
                    if !caller.role.has_permission(perm) {
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

### Service Usage

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
    pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
        HARNESS_MANAGE.evaluate(caller)?;
        // ... existing logic using caller.org_id ...
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        HARNESS_DANGEROUS.evaluate(caller)?;
        // ...
    }
}
```

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
GET /v1/sessions/config
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
pub async fn harness_config(org: ResolvedOrg) -> Json<ConfigResponse> {
    let caller = Caller::from(&org);
    Json(ConfigResponse {
        policies: evaluate_policies(&caller, &[HARNESS_MANAGE, HARNESS_DANGEROUS]),
    })
}
```

## Migration Path

### Phase 1 (this spec)
- Permission enum, Rule enum, Policy struct
- Caller context type
- Role → permission mapping (hardcoded)
- Service-layer enforcement
- Config endpoints for UI
- No DB changes

### Phase 2 (future)
- `user_permissions` table for per-user grants beyond role defaults
- `UserIsResourceOwner` rule (needs `created_by` on resources)
- `IsPlatformUser` rule with allowlist
- Admin UI for permission management

## Files

| File | Purpose |
|------|---------|
| `crates/core/src/permissions.rs` | Permission enum, Rule, Policy, Caller, evaluation logic |
| `crates/server/src/services/*.rs` | Policy enforcement at method entry |
| `crates/server/src/api/*_config.rs` | Config endpoints returning policy results |
| `crates/server/src/auth/middleware.rs` | Caller extraction from ResolvedOrg |
