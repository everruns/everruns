# Policy Coverage Plan

## Overview

Wire policies into all services, add view/manage split, platform user rule for durable, UI shared helper, update specs to mandate policies for new features.

## 1. Core: New permissions + IsPlatformUser rule

**File: `crates/core/src/permissions.rs`**

Add 4 view permissions:
```
OrgHarnessesView       → org:harnesses:view
OrgLlmProvidersView    → org:llm-providers:view
OrgSettingsView        → org:settings:view
OrgMembersView         → org:members:view
```

Update role mapping — all 3 roles get all 4 view permissions (Members can view everything).

Add `IsPlatformUser` to `Rule` enum.

Add `is_platform_user: bool` to `Caller`. Update `Caller::internal()` → `is_platform_user: true`.

Update `Policy::evaluate()`:
```rust
Rule::IsPlatformUser => {
    if !caller.is_platform_user {
        return Err(PolicyError::denied(self.id, "platform_user"));
    }
}
```

Rename `PolicyConfigResponse` → `ResourceConfigResponse` (same shape: `{ policies: HashMap<String, bool> }`).

## 2. Platform user resolution

**File: `crates/server/src/auth/config.rs`** — load `PLATFORM_USER_EMAILS` env var (comma-separated) into `AuthConfig`.

**File: `crates/server/src/api/common.rs`** — shared `caller_from_org(org, platform_emails)` that checks if user email is in platform list → sets `is_platform_user`.

For durable: build `Caller` from `AuthUser` + platform check. Define `DURABLE_MANAGE` policy with `Rule::IsPlatformUser`. Replace `AdminUser` extractor.

## 3. Service layer: Add Caller + policies

Each service gets `Caller` param replacing `org_id`, policy consts, `#[policy]` on every method.

### AgentService
- VIEW: `agent.view` → `OrgAgentsManage` (get, get_by_public_id, list)
- MANAGE: `agent.manage` → `OrgAgentsManage` (create, update, copy, delete, upsert)

### AppService
- VIEW: `app.view` → `OrgAgentsManage` (get, list)
- MANAGE: `app.manage` → `OrgAgentsManage` (create, update)
- DANGEROUS: `app.dangerous` → `OrgAgentsManage` + `OrgSettingsManage` (delete, publish, unpublish)

### SessionService
- VIEW: `session.view` → `OrgSessionsManage` (get, list, stats)
- MANAGE: `session.manage` → `OrgSessionsManage` (create, update, update_status, delete, get_or_create_chat, pin, unpin)

### McpServerService (Members allowed — building blocks)
- VIEW: `mcp_server.view` → `OrgAgentsManage` (get, list, list_active, list_active_with_tools, get_tools, get_cached_tools, get_batch_with_tools, resolve_by_prefix)
- MANAGE: `mcp_server.manage` → `OrgAgentsManage` (create, update, delete, refresh_tools, decrypt_api_key)

### LlmProviderService
- VIEW: `llm_provider.view` → `OrgLlmProvidersView` (get, list)
- MANAGE: `llm_provider.manage` → `OrgLlmProvidersManage` (create, update, delete)

### LlmModelService
- VIEW: `llm_model.view` → `OrgLlmProvidersView` (get_with_provider, list_for_provider, list_all, list_all_with_filters, get_default)
- MANAGE: `llm_model.manage` → `OrgLlmProvidersManage` (create, update, delete)

### SkillService
- VIEW: `skill.view` → `OrgAgentsManage` (get, list, get_content)
- MANAGE: `skill.manage` → `OrgAgentsManage` (create, create_from_archive, update, delete)

### HarnessService (update existing)
- VIEW: `harness.view` → `OrgHarnessesView` (get, list — currently `HARNESS_MANAGE`, downgrade)
- MANAGE/DANGEROUS: unchanged

### Durable (new)
- `DURABLE_MANAGE` → `IsPlatformUser` (all durable endpoints)

## 4. API layer: Update handlers + config endpoints

For each resource API file:
1. Use shared `caller_from_org()` from `common.rs`
2. Pass `Caller` to service methods
3. Use `.map_policy_or_internal()` on results
4. Add `GET /v1/{resource}/config` → `ResourceConfigResponse`

New config endpoints: agents, apps, sessions, mcp-servers, llm-providers, llm-models, skills.
Update: harnesses config (add `harness.view`, use `ResourceConfigResponse`).

Durable: replace `AdminUser` with `Caller` + `DURABLE_MANAGE` policy.

## 5. Internal callers (gRPC, workers)

All `Caller::internal(org_id)` already gets Owner role + `is_platform_user: true`.
Update gRPC service methods to pass `Caller::internal()` instead of raw `org_id`.

## 6. UI: Shared policy helper

### Types (`apps/ui/src/lib/api/types.ts`)
```ts
export interface ResourceConfigResponse {
  policies: Record<string, boolean>;
}
```

### Hook (`apps/ui/src/hooks/use-policies.ts`)
```ts
export function usePolicies(resource: string) {
  // Fetches GET /v1/{resource}/config
  // Returns { can(policyId): boolean, isLoading, ... }
}
```

### Component (`apps/ui/src/components/policy-gate.tsx`)
```tsx
export function PolicyGate({ resource, policy, fallback?, children }) {
  // Renders children only if policy passes
}
```

### Query keys — add `policies.config(resource)`.

### Wire into UI pages — harnesses as example, others follow pattern.

## 7. Specs updates

### `specs/permissions.md`
- Add view permissions to table
- Add `IsPlatformUser` rule (move from "future" to current)
- Add `ResourceConfigResponse` pattern
- Add durable policy
- **Add section: "Policy Requirements" — all new service methods MUST have `#[policy]` with `Caller`**

### `specs/code-organization.md`
Add rule: "All service methods MUST have `#[policy]` enforcement with `Caller` parameter. New features without policies will not pass review."

### `specs/apis.md`
Document config endpoints.

### `specs/durable-execution-engine.md`
Document platform user access replacing AdminUser.

## 8. Tests

### Core unit tests
- View permissions in role mapping (all roles get view)
- `IsPlatformUser` rule (pass when true, fail when false)
- `is_platform_user` field on Caller

### Integration tests
- Need TestServer role-switching support to test 403 responses
- Test at least: Member gets 403 on harness create, Admin gets 403 on harness delete

## Execution order

1. Core permissions (view perms, IsPlatformUser, Caller field, ResourceConfigResponse)
2. Platform user loading in auth config
3. Shared caller_from_org in common.rs
4. HarnessService: add HARNESS_VIEW, update get/list
5-11. Remaining services: add Caller + policies (agent, app, session, mcp, llm_provider, llm_model, skill)
12. API handlers: update all to pass Caller + add config endpoints
13. Durable: replace AdminUser with IsPlatformUser policy
14. gRPC/internal: update to Caller::internal
15. UI: usePolicies + PolicyGate + wire harnesses
16. Specs: update permissions, code-organization, apis, durable
17. Tests
