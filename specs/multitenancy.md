# Multitenancy Specification

## Abstract

This document defines the multitenancy model for Everruns, enabling organization-based resource isolation. Organizations are the administrative unit for ownership, membership, and billing. Users can belong to multiple organizations.

## Decisions Log

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Membership model | Members only (no roles) | Simplicity; roles can be added later |
| Multi-org support | Yes | Users switch between orgs in UI |
| Personal namespace | No | Org-only model; SaaS auto-creates org on signup |
| API org identifier | Path-based `/v1/orgs/{org_public_id}/...` | Explicit, debuggable, RESTful |
| Org ID format | BIGINT internal + TEXT public_id | Performance + security |
| Resource sharing | No cross-org sharing | Isolation by default |
| API keys | Org-scoped | Limited blast radius if leaked |
| InMemory storage | Supports multitenancy | Consistency across modes |
| Default org | Seeded on startup | No "first boot" concept |
| Migration | Reset (no backward compat) | Clean slate |

## Requirements

### Organization Entity

| Field | Type | Description |
|-------|------|-------------|
| `org_id` | BIGINT | Internal primary key (auto-increment) |
| `public_id` | TEXT | External identifier: `org_<uuid-hex-32>` |
| `name` | TEXT | Display name |
| `created_at` | TIMESTAMPTZ | Creation time |
| `updated_at` | TIMESTAMPTZ | Last modification time |

**Public ID Format:**
- Pattern: `^org_[0-9a-f]{32}$`
- Example: `org_2f3c1b3e6a9d4c6f8a1d4e9c9b7f21a0`
- Generated at creation time (UUIDv4, lowercase hex, no dashes)
- Not derived from `org_id`

**Security Rules:**
- `org_id` MUST NOT appear in APIs, URLs, logs, or error messages
- APIs accept and return only `public_id`
- Authorization enforced using resolved `org_id`

### Organization Membership

| Field | Type | Description |
|-------|------|-------------|
| `org_id` | BIGINT | Organization reference |
| `user_id` | UUID | User reference |
| `created_at` | TIMESTAMPTZ | Join time |

**Constraints:**
- Primary key: `(org_id, user_id)`
- User can belong to multiple organizations
- No roles for now (all members have equal access)

### Resource Scoping

| Resource | Scope | Implementation |
|----------|-------|----------------|
| Agent | Per-org | `org_id` FK on `agents` table |
| Session | Inherits from Agent | No direct FK; scoped via agent join |
| Messages/Events | Inherits from Session | No direct FK; scoped via session→agent join |
| LLM Provider | Per-org | `org_id` FK on `llm_providers` table |
| LLM Model | Per-org | `org_id` FK on `llm_models` table |
| API Key | Per-org | `org_id` FK on `api_keys` table |
| Capabilities | Global | No `org_id`; system-defined |
| MCP Servers | Per-org | `org_id` FK (future) |
| Usage Tracking | Per-org | Aggregated by `org_id` |

**Query Rules:**
- ALL database queries for org-scoped resources MUST include `WHERE org_id = $org_id`
- No exceptions, even for UUID lookups
- Returns 404 (not 403) when resource exists but belongs to different org

### Database Schema

```sql
-- Organizations
CREATE TABLE organizations (
    org_id BIGSERIAL PRIMARY KEY,
    public_id TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT organizations_public_id_format
        CHECK (public_id ~ '^org_[0-9a-f]{32}$')
);

CREATE INDEX idx_organizations_public_id ON organizations(public_id);

-- Organization Members
CREATE TABLE organization_members (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, user_id)
);

CREATE INDEX idx_organization_members_user_id ON organization_members(user_id);

-- Add org_id to existing tables
ALTER TABLE agents ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id);
ALTER TABLE llm_providers ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id);
ALTER TABLE llm_models ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id);
ALTER TABLE api_keys ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id);

-- Composite indexes for org-scoped queries
CREATE INDEX idx_agents_org_id ON agents(org_id);
CREATE INDEX idx_llm_providers_org_id ON llm_providers(org_id);
CREATE INDEX idx_llm_models_org_id ON llm_models(org_id);
CREATE INDEX idx_api_keys_org_id ON api_keys(org_id);
```

### API Design

**Path Structure:**
```
/v1/orgs/{org_public_id}/agents
/v1/orgs/{org_public_id}/agents/{agent_id}
/v1/orgs/{org_public_id}/agents/{agent_id}/sessions
/v1/orgs/{org_public_id}/agents/{agent_id}/sessions/{session_id}
/v1/orgs/{org_public_id}/agents/{agent_id}/sessions/{session_id}/messages
/v1/orgs/{org_public_id}/agents/{agent_id}/sessions/{session_id}/sse
/v1/orgs/{org_public_id}/llm-providers
/v1/orgs/{org_public_id}/llm-providers/{provider_id}
/v1/orgs/{org_public_id}/llm-models
/v1/orgs/{org_public_id}/api-keys
```

**Global Endpoints (no org scope):**
```
/health
/v1/auth/*
/v1/users/me
/v1/capabilities
```

**User Info Response Enhancement:**
```json
// GET /v1/users/me (or /v1/auth/me)
{
  "id": "...",
  "email": "user@example.com",
  "name": "User Name",
  "organizations": [
    {
      "public_id": "org_2f3c1b3e6a9d4c6f8a1d4e9c9b7f21a0",
      "name": "Acme Corp"
    },
    {
      "public_id": "org_00000000000000000000000000000001",
      "name": "Default Organization"
    }
  ]
}
```

### Authentication Context

**Enhanced AuthUser:**
```rust
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub roles: Vec<String>,
    pub auth_method: AuthMethod,
    pub organizations: Vec<OrgMembership>,  // NEW
}

pub struct OrgMembership {
    pub org_id: i64,        // Internal, for DB queries
    pub public_id: String,  // External, for API responses
    pub name: String,
}
```

**Org Context Extractor:**
```rust
/// Extracts organization from path parameter and validates user membership.
/// Returns 404 if org not found or user is not a member.
pub struct OrgContext {
    pub org_id: i64,        // For DB queries
    pub public_id: String,  // For API responses
}

impl<S> FromRequestParts<S> for OrgContext {
    // 1. Extract org_public_id from path
    // 2. Look up organization by public_id
    // 3. Verify AuthUser is member of org
    // 4. Return OrgContext or 404
}
```

**API Key Context:**
```rust
// API keys are org-scoped; key lookup returns org context directly
pub struct ApiKeyContext {
    pub user_id: Uuid,
    pub org_id: i64,
    pub scopes: Vec<String>,
}
```

### Storage Layer Changes

**Repository Method Signatures:**
```rust
// All org-scoped methods require org_id as first parameter

// AgentRepository
fn list_agents(&self, org_id: i64, pagination: Pagination) -> Result<(Vec<AgentRow>, u32)>;
fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<AgentRow>>;
fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow>;
fn update_agent(&self, org_id: i64, agent_id: Uuid, input: UpdateAgent) -> Result<AgentRow>;
fn delete_agent(&self, org_id: i64, agent_id: Uuid) -> Result<()>;

// SessionRepository (org via agent join)
fn list_sessions(&self, org_id: i64, agent_id: Uuid, pagination: Pagination) -> Result<...>;
fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<SessionRow>>;

// LlmProviderRepository
fn list_providers(&self, org_id: i64) -> Result<Vec<LlmProviderRow>>;
fn get_provider(&self, org_id: i64, provider_id: Uuid) -> Result<Option<LlmProviderRow>>;

// LlmModelRepository
fn list_models(&self, org_id: i64) -> Result<Vec<LlmModelRow>>;
fn get_model(&self, org_id: i64, model_id: Uuid) -> Result<Option<LlmModelRow>>;

// ApiKeyRepository
fn list_api_keys(&self, org_id: i64, user_id: Uuid) -> Result<Vec<ApiKeyRow>>;
fn create_api_key(&self, org_id: i64, user_id: Uuid, input: CreateApiKey) -> Result<...>;
```

**Query Examples:**
```sql
-- List agents for org
SELECT * FROM agents WHERE org_id = $1 ORDER BY created_at DESC;

-- Get agent (must match both org_id and agent_id)
SELECT * FROM agents WHERE org_id = $1 AND id = $2;

-- Get session (join to verify org ownership)
SELECT s.* FROM sessions s
JOIN agents a ON s.agent_id = a.id
WHERE a.org_id = $1 AND s.id = $2;

-- Get events (join through session and agent)
SELECT e.* FROM events e
JOIN sessions s ON e.session_id = s.id
JOIN agents a ON s.agent_id = a.id
WHERE a.org_id = $1 AND e.session_id = $2
ORDER BY e.sequence;
```

### Seeds

**Default Organization:**
```rust
// Well-known IDs for seeded data
pub const DEFAULT_ORG_ID: i64 = 1;
pub const DEFAULT_ORG_PUBLIC_ID: &str = "org_00000000000000000000000000000001";

pub async fn seed_default_organization(db: &Database) -> Result<()> {
    // Insert default organization (idempotent)
    sqlx::query!(
        r#"
        INSERT INTO organizations (org_id, public_id, name)
        VALUES ($1, $2, $3)
        ON CONFLICT (org_id) DO NOTHING
        "#,
        DEFAULT_ORG_ID,
        DEFAULT_ORG_PUBLIC_ID,
        "Default Organization"
    ).execute(db).await?;
    Ok(())
}
```

**Seeded Resources:**
- Default organization is created first
- All seeded providers, models, agents belong to default org
- In auth mode `none`, anonymous user is auto-added to default org

**Seed Order:**
1. `seed_default_organization()`
2. `seed_llm_providers()` (with `org_id = DEFAULT_ORG_ID`)
3. `seed_llm_models()` (with `org_id = DEFAULT_ORG_ID`)
4. `seed_agents()` (with `org_id = DEFAULT_ORG_ID`)

### InMemory Storage

InMemory storage (DEV_MODE) supports multitenancy:
- Default organization pre-created on initialization
- All org-scoped methods require `org_id` parameter
- Same API contract as PostgreSQL storage

```rust
impl InMemoryStorage {
    pub fn new() -> Self {
        let storage = Self::default();
        // Pre-create default organization
        storage.organizations.insert(DEFAULT_ORG_ID, Organization {
            org_id: DEFAULT_ORG_ID,
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            ..
        });
        storage
    }
}
```

### Worker Integration

**gRPC Context:**
- Worker requests include `org_id` in metadata
- Control plane validates org ownership before returning context
- Turn context (`GetTurnContext`) scoped to org

```protobuf
message GetTurnContextRequest {
    string session_id = 1;
    int64 org_id = 2;  // NEW: Required for validation
}
```

### UI Changes

**Organization Selector:**
- Dropdown in sidebar/header showing current org
- Lists all organizations user belongs to
- Selection persisted in localStorage
- API calls include selected org in path

**State Management:**
```typescript
interface OrgState {
  currentOrg: Organization | null;
  organizations: Organization[];
  setCurrentOrg: (org: Organization) => void;
}

// All API calls use current org
const agents = useAgents(currentOrg.public_id);
```

**URL Structure:**
```
/orgs/{org_public_id}/agents
/orgs/{org_public_id}/agents/{agent_id}
/orgs/{org_public_id}/settings
```

### Usage Tracking

Usage is aggregated per organization:
```sql
-- Add org_id to usage tracking
ALTER TABLE usage_records ADD COLUMN org_id BIGINT NOT NULL;

-- Query usage by org
SELECT
    org_id,
    SUM(input_tokens) as total_input,
    SUM(output_tokens) as total_output
FROM usage_records
WHERE org_id = $1 AND created_at >= $2
GROUP BY org_id;
```

### Error Handling

**404 vs 403:**
- Resource exists but wrong org → 404 (prevents enumeration)
- User not member of org → 404 (prevents org discovery)
- Invalid public_id format → 400 Bad Request

**Error Messages:**
```rust
// Good - no information leakage
ApiError::NotFound("Agent not found")
ApiError::NotFound("Organization not found")

// Bad - reveals existence
ApiError::Forbidden("You don't have access to this agent")
```

## Implementation Phases

### Phase 1: Schema & Core Types ✅
- [x] Add `organizations` table
- [x] Add `organization_members` table
- [x] Add `org_id` FK to `agents`, `llm_providers`, `llm_models`, `api_keys`
- [x] Create core `Organization` and `OrganizationMember` types
- [x] Implement `OrgId` newtype with `public_id` generation

### Phase 2: Storage Layer ✅
- [x] Create `OrganizationRepository`
- [x] Create `OrganizationMemberRepository`
- [x] Update `AgentRepository` to require `org_id`
- [x] Update `SessionRepository` to validate org via agent
- [x] Update `EventRepository` to validate org via session→agent
- [x] Update `LlmProviderRepository` to require `org_id`
- [x] Update `LlmModelRepository` to require `org_id`
- [x] Update `ApiKeyRepository` to require `org_id`
- [x] Update InMemory storage with same changes

### Phase 3: Auth & Extractors ✅
- [x] Enhance `AuthUser` with `organizations` list
- [x] Create `OrgContext` extractor
- [x] Update API key auth to include org context
- [x] Update anonymous auth to use default org

### Phase 4: API Routes ✅
- [x] Add org CRUD endpoints (`/v1/orgs`)
- [x] Migrate all agent routes to `/v1/orgs/{org}/agents/...`
- [x] Migrate LLM provider routes to `/v1/orgs/{org}/llm-providers/...`
- [x] Migrate LLM model routes to `/v1/orgs/{org}/llm-models/...`
- [x] Migrate API key routes to `/v1/orgs/{org}/api-keys/...`
- [x] Migrate capabilities routes to `/v1/orgs/{org}/capabilities/...`
- [x] Migrate MCP server routes to `/v1/orgs/{org}/mcp-servers/...`
- [x] Migrate session files routes to `/v1/orgs/{org}/agents/{agent}/sessions/{session}/fs/...`
- [x] Update OpenAPI spec

### Phase 5: Seeds ✅
- [x] Update seed system to create default org first
- [x] Update provider seeds with `org_id`
- [x] Update model seeds with `org_id`
- [x] Update agent seeds with `org_id`

### Phase 6: Worker Integration ✅
- [x] Update gRPC protocol with `org_id`
- [x] Update worker context validation
- [x] Update durable execution with org context

### Phase 7: UI ✅
- [x] Add organization selector component (sidebar dropdown)
- [x] Update all API calls with org path
- [x] Add OrgProvider context with useOrg() hook
- [x] Persist selected org in localStorage
- [ ] Add org management page (view members) - deferred to future

### Phase 8: Usage & Cleanup ✅
- [x] Add `org_id` to usage tracking
- [x] Update usage aggregation queries
- [x] Remove any remaining global resource access
- [x] Security audit for org isolation

## Implementation Notes

### OrgContext Extractor
The `OrgContext` extractor (`control-plane/src/api/org_context.rs`) extracts org from URI path directly since Axum's `Path<T>` extractor consumes the request body. It validates membership against the authenticated user's organizations.

### Anonymous Auth Mode (AUTH_MODE=none)
When auth is disabled, the UI uses a hardcoded default organization:
- `public_id`: `org_00000000000000000000000000000001`
- `name`: "Default Organization"

The frontend `OrgProvider` handles this by returning `DEFAULT_ORG_MEMBERSHIP` when `!requiresAuth`.

### System-Wide Resources
Some resources remain system-wide (not org-scoped):
- `/v1/durable/*` - Durable execution workers and workflows (infrastructure-level)
- `/health` - Health check endpoint
- `/v1/auth/*` - Authentication endpoints

### UI Organization Selector
Located in sidebar (`apps/ui/src/components/layout/sidebar.tsx`). Shows current org with dropdown to switch between user's organizations. Uses Base UI's `DropdownMenu` component.

## Future Considerations

**Not in scope for v1:**
- Organization roles (admin, member, viewer)
- Invitations and onboarding flow
- Org-level settings (allowed providers, limits)
- Cross-org resource sharing
- Organization deletion (with cascade)
- Audit logging per org
