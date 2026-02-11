# OSS Changes for SaaS Repository Support

> Companion to `specs/saas-repo.md`. Describes exact changes needed in the OSS
> Everruns repo to support a private SaaS wrapper repo. All changes are
> general-purpose and benefit OSS users (pluggable auth, external identity
> providers, etc.).

## Overview

| Area | Change | Size | Files |
|------|--------|------|-------|
| Auth backend trait | Extract pluggable `AuthBackend` trait | ~60 lines new | `auth/backend.rs`, `auth/mod.rs` |
| Builtin backend | Move existing logic into `BuiltinAuthBackend` | ~80 lines moved | `auth/builtin.rs` |
| AuthState refactor | Hold `Box<dyn AuthBackend>` instead of hardcoded services | ~20 lines changed | `auth/middleware.rs` |
| Server entrypoint | Extract `run()` into `lib.rs`, keep `main.rs` thin | ~40 lines moved | `lib.rs`, `main.rs` |
| Extra routes hook | Accept optional extra `Router` for SaaS-only endpoints | ~5 lines | `lib.rs` |
| Storage: external_id | Add `external_id` to users + organizations | ~15 lines | migration, `models.rs`, `repositories.rs`, `memory.rs`, `backend.rs` |
| Storage: new queries | `get_user_by_external_id`, `upsert_org_by_external_id` | ~40 lines | `repositories.rs`, `memory.rs`, `backend.rs` |
| UI: export context | Export `AuthContext` and `AuthContextValue` | ~3 lines changed | `auth-provider.tsx` |
| UI: export org context | Export `OrgContext` and `OrgContextValue` | ~3 lines changed | `org-provider.tsx` |
| UI: package exports | Add `exports` field to `package.json` | ~10 lines | `package.json` |

**Total: ~280 lines changed/added across ~15 files. Zero breaking changes.**

---

## 1. Auth Backend Trait

**New file: `crates/server/src/auth/backend.rs`**

```rust
// Pluggable authentication backend
// Decision: Trait allows external auth providers (PropelAuth, Auth0, Keycloak)
// without modifying OSS code. OSS ships BuiltinAuthBackend as the default.

use async_trait::async_trait;
use axum::Router;
use super::middleware::{AuthUser, AuthError};

/// Authentication backend trait.
///
/// Implementations validate credentials and return an `AuthUser`.
/// The OSS default is `BuiltinAuthBackend` (JWT + password + API key).
/// SaaS wrappers provide their own implementation (e.g., PropelAuth).
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// Validate a Bearer token (JWT or opaque) and return the authenticated user.
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError>;

    /// Validate an API key and return the authenticated user.
    /// API keys use the `evr_` prefix and SHA-256 hashing.
    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError>;

    /// Return auth-specific HTTP routes (login, register, OAuth callbacks, API key CRUD).
    /// Returns `None` if auth is handled externally (e.g., PropelAuth hosted UI).
    fn auth_routes(&self) -> Option<Router>;

    /// Return the auth configuration for the `/v1/auth/config` endpoint.
    /// This tells the UI which auth features are available.
    fn auth_config_response(&self) -> AuthConfigResponse;
}
```

**Why trait instead of enum dispatch:** New auth providers can be added in external crates without touching OSS. Enum dispatch would require modifying the OSS `AuthMode` enum for each new provider.

---

## 2. Builtin Auth Backend

**New file: `crates/server/src/auth/builtin.rs`**

Moves existing logic from `middleware.rs` (`validate_jwt_token`, `validate_api_key`, `fetch_user_organizations`) into a struct that implements `AuthBackend`.

```rust
// Built-in authentication backend (JWT + password + OAuth + API keys)
// Decision: Default for OSS. All existing behavior preserved.

use super::backend::AuthBackend;
use super::config::AuthConfig;
use super::jwt::JwtService;
use super::routes;
use crate::storage::StorageBackend;

pub struct BuiltinAuthBackend {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
}

impl BuiltinAuthBackend {
    pub fn new(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let jwt_service = Arc::new(JwtService::new(config.jwt.clone()));
        Self { config, jwt_service, db }
    }
}

#[async_trait]
impl AuthBackend for BuiltinAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        // Move existing validate_jwt_token() logic here (middleware.rs:202-241)
    }

    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError> {
        // Move existing validate_api_key() logic here (middleware.rs:244-318)
    }

    fn auth_routes(&self) -> Option<Router> {
        // Return existing routes::routes() (routes.rs:143-162)
        Some(routes::routes(self.clone()))
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        // Move existing get_auth_config logic here (routes.rs)
    }
}
```

**Key:** The two private functions `validate_jwt_token()` and `validate_api_key()` in `middleware.rs` (lines 202-318) become methods on `BuiltinAuthBackend`. Their logic is unchanged.

---

## 3. AuthState Refactor

**File: `crates/server/src/auth/middleware.rs`**

### Current (lines 129-146)

```rust
pub struct AuthState {
    pub config: AuthConfig,
    pub jwt_service: Arc<JwtService>,
    pub db: Arc<StorageBackend>,
}
```

### New

```rust
pub struct AuthState {
    pub config: AuthConfig,
    pub backend: Arc<dyn AuthBackend>,
}

impl AuthState {
    pub fn new(config: AuthConfig, backend: Arc<dyn AuthBackend>) -> Self {
        Self { config, backend }
    }

    /// Convenience: create with built-in backend (OSS default)
    pub fn builtin(config: AuthConfig, db: Arc<StorageBackend>) -> Self {
        let backend = Arc::new(BuiltinAuthBackend::new(config.clone(), db));
        Self { config, backend }
    }
}
```

### Extractor change (lines 164-199)

The `extract_auth_user()` function delegates to the backend:

```rust
async fn extract_auth_user(
    parts: &mut Parts,
    auth_state: &AuthState,
) -> Result<AuthUser, AuthError> {
    // None mode — unchanged (line 169-171)
    if auth_state.config.mode == AuthMode::None {
        return Ok(AuthUser::anonymous());
    }

    // Bearer token
    if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
        let auth_str = auth_header.to_str()
            .map_err(|_| AuthError::unauthorized("Invalid authorization header"))?;

        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            return auth_state.backend.validate_token(token).await;  // <-- trait call
        }

        if auth_str.starts_with(API_KEY_PREFIX) || auth_str.starts_with("ApiKey ") {
            let api_key = auth_str.strip_prefix("ApiKey ").unwrap_or(auth_str);
            return auth_state.backend.validate_api_key(api_key).await;  // <-- trait call
        }
    }

    // Cookie fallback
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get("access_token") {
        return auth_state.backend.validate_token(cookie.value()).await;  // <-- trait call
    }

    Err(AuthError::unauthorized("Authentication required"))
}
```

**Impact:** Only 3 lines change in the extractor (direct function calls → `backend.method()` calls). All other extractors (`OptionalAuthUser`, `AdminUser`, `OrgContext`, `ResolvedOrg`) are unchanged — they call `AuthUser::from_request_parts()` which calls `extract_auth_user()`.

---

## 4. Server Entrypoint Extraction

**Goal:** Move the server bootstrap from `main.rs` into a reusable `run()` function in `lib.rs` so the SaaS binary can call it with a custom `AuthBackend` and extra routes.

### File: `crates/server/src/lib.rs`

Add:

```rust
pub mod server;  // new module

// Re-exports for SaaS binary
pub use server::{run, ServerConfig};
pub use auth::backend::AuthBackend;
pub use auth::builtin::BuiltinAuthBackend;
```

### New file: `crates/server/src/server.rs`

Extract the body of `main()` (lines 76-583) into:

```rust
/// Server configuration loaded from environment
pub struct ServerConfig {
    pub dev_mode: bool,
    pub no_migrations: bool,
    pub database_url: Option<String>,
    pub api_prefix: String,
    pub cors_origins: Vec<HeaderValue>,
    pub addr: String,
    pub grpc_addr: String,
}

impl ServerConfig {
    pub fn from_env() -> Self { /* existing env var loading */ }
}

/// Start the Everruns server.
///
/// `auth_backend` — authentication implementation (use `BuiltinAuthBackend` for OSS default)
/// `extra_routes` — additional routes merged into the API router (billing, etc.)
pub async fn run(
    config: ServerConfig,
    auth_backend: Arc<dyn AuthBackend>,
    extra_routes: Option<Router>,
) -> Result<()> {
    // Lines 80-583 of current main.rs, with these changes:
    // 1. Replace `auth::AuthState::new(auth_config, db)` with `AuthState::new(auth_config, auth_backend)`
    // 2. Replace `auth::routes(auth_state)` with `auth_backend.auth_routes()`
    // 3. Merge extra_routes into api_routes if Some
    // 4. Everything else unchanged
}
```

### File: `crates/server/src/main.rs`

Becomes thin:

```rust
mod grpc_service;
mod seed;

use clap::Parser;
use everruns_server::{ServerConfig, BuiltinAuthBackend, run};
use everruns_server::auth::AuthConfig;
use everruns_server::storage::StorageBackend;

#[derive(Parser)]
#[command(name = "everruns-server")]
struct Args {
    #[arg(long)]
    no_migrations: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut config = ServerConfig::from_env();
    config.no_migrations = args.no_migrations;

    let auth_config = AuthConfig::from_env();
    // Note: db is created inside run() — BuiltinAuthBackend needs db reference.
    // Two options:
    //   A. Pass a factory closure: run(config, |db| BuiltinAuthBackend::new(auth_config, db))
    //   B. Create db before run(), pass it in
    // Option B is cleaner:
    let db = /* create StorageBackend from config, same as current lines 106-159 */;
    let auth = Arc::new(BuiltinAuthBackend::new(auth_config, db.clone()));

    run(config, auth, None).await
}
```

**Design decision:** Whether to create `StorageBackend` inside or outside `run()`. Recommend **inside** `run()` with a factory pattern:

```rust
pub async fn run(
    config: ServerConfig,
    auth_factory: impl FnOnce(Arc<StorageBackend>) -> Arc<dyn AuthBackend>,
    extra_routes: Option<Router>,
) -> Result<()> {
    let db = /* create StorageBackend */;
    let auth_backend = auth_factory(db.clone());
    let auth_state = AuthState::new(auth_config, auth_backend);
    // ...
}
```

This keeps `StorageBackend` creation centralized and avoids duplicating migration logic in the SaaS binary.

**OSS main.rs:**
```rust
run(config, |db| Arc::new(BuiltinAuthBackend::new(auth_config, db)), None).await
```

**SaaS main.rs:**
```rust
run(config, |db| Arc::new(PropelAuthBackend::new(saas_config, db)), Some(billing_routes)).await
```

---

## 5. Auth Module Exports

**File: `crates/server/src/auth/mod.rs`**

### Current

```rust
pub mod api_key;
pub mod config;
pub mod jwt;
pub mod middleware;
pub mod oauth;
pub mod routes;

pub use config::AuthConfig;
pub use middleware::{AuthState, AuthUser, OrgContext, ResolvedOrg};
pub use routes::routes;
```

### New

```rust
pub mod api_key;
pub mod backend;     // NEW
pub mod builtin;     // NEW
pub mod config;
pub mod jwt;
pub mod middleware;
pub mod oauth;
pub mod routes;

pub use backend::AuthBackend;                              // NEW
pub use builtin::BuiltinAuthBackend;                       // NEW
pub use config::AuthConfig;
pub use middleware::{AuthError, AuthState, AuthUser, AuthMethod, OrgContext, ResolvedOrg};
pub use routes::routes;
```

Note: `AuthError` and `AuthMethod` are now also exported (needed by `AuthBackend` implementors).

---

## 6. Storage: External Identity Support

### 6a. Migration

**New file: `crates/server/migrations/004_external_identity.sql`**

```sql
-- Support for external identity providers (PropelAuth, Auth0, etc.)
-- The external_id column maps external provider user/org IDs to internal IDs.
-- OSS users: unused (NULL). SaaS: populated by auth backend sync.

ALTER TABLE users ADD COLUMN external_id TEXT UNIQUE;
ALTER TABLE organizations ADD COLUMN external_id TEXT UNIQUE;

CREATE INDEX idx_users_external_id ON users(external_id) WHERE external_id IS NOT NULL;
CREATE INDEX idx_organizations_external_id ON organizations(external_id) WHERE external_id IS NOT NULL;
```

### 6b. Models

**File: `crates/server/src/storage/models.rs`**

Add `external_id` field to:

```rust
// UserRow (line 56)
pub struct UserRow {
    // ... existing fields ...
    pub external_id: Option<String>,  // NEW
}

// OrganizationRow (line 14)
pub struct OrganizationRow {
    // ... existing fields ...
    pub external_id: Option<String>,  // NEW
}

// CreateUserRow (line 107)
pub struct CreateUserRow {
    // ... existing fields ...
    pub external_id: Option<String>,  // NEW
}
```

### 6c. New Storage Methods

**File: `crates/server/src/storage/repositories.rs`**

```rust
/// Get user by external identity provider ID
pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT * FROM users WHERE external_id = $1"
    )
    .bind(external_id)
    .fetch_optional(&self.pool)
    .await?;
    Ok(row)
}

/// Get organization by external identity provider ID
pub async fn get_organization_by_external_id(&self, external_id: &str) -> Result<Option<OrganizationRow>> {
    let row = sqlx::query_as::<_, OrganizationRow>(
        "SELECT * FROM organizations WHERE external_id = $1"
    )
    .bind(external_id)
    .fetch_optional(&self.pool)
    .await?;
    Ok(row)
}

/// Upsert organization by external ID (for external auth provider sync)
pub async fn upsert_org_by_external_id(
    &self,
    external_id: &str,
    name: &str,
) -> Result<OrganizationRow> {
    let row = sqlx::query_as::<_, OrganizationRow>(
        "INSERT INTO organizations (public_id, name, external_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (external_id) DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
         RETURNING *"
    )
    .bind(generate_public_id("org"))  // from id-schema
    .bind(name)
    .bind(external_id)
    .fetch_one(&self.pool)
    .await?;
    Ok(row)
}

/// Ensure user is a member of organization (idempotent)
pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id)
         VALUES ($1, $2)
         ON CONFLICT (org_id, user_id) DO NOTHING"
    )
    .bind(org_id)
    .bind(user_id)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

Mirror these in `memory.rs` and `backend.rs` (delegation layer).

---

## 7. UI: Export Auth Context

**File: `apps/ui/src/providers/auth-provider.tsx`**

### Current (line 11, 28)

```typescript
interface AuthContextValue { ... }
const AuthContext = createContext<AuthContextValue | undefined>(undefined);
```

### New

```typescript
export interface AuthContextValue { ... }   // add export
export const AuthContext = createContext<AuthContextValue | undefined>(undefined);  // add export
```

Exporting `AuthContext` allows the SaaS `propelauth-provider.tsx` to provide values to the same context, so all downstream `useAuth()` calls work unchanged.

---

## 8. UI: Export Org Context

**File: `apps/ui/src/providers/org-provider.tsx`**

### Current (line 26, 37)

```typescript
interface OrgContextValue { ... }
const OrgContext = createContext<OrgContextValue | undefined>(undefined);
```

### New

```typescript
export interface OrgContextValue { ... }   // add export
export const OrgContext = createContext<OrgContextValue | undefined>(undefined);  // add export
```

---

## 9. UI: Package Exports

**File: `apps/ui/package.json`**

Add `exports` field so the SaaS UI can import from `@everruns/ui/providers/auth-provider`:

```jsonc
{
  "name": "everruns-ui",
  // ... existing fields ...
  "exports": {
    ".": "./src/index.ts",
    "./providers/*": "./src/providers/*.tsx",
    "./components/*": "./src/components/*.tsx",
    "./hooks/*": "./src/hooks/*.ts",
    "./lib/*": "./src/lib/*.ts",
    "./lib/api/*": "./src/lib/api/*.ts",
    "./app/*": "./src/app/*"
  }
}
```

**Note:** The `name` field stays `everruns-ui` (no scope change needed). The SaaS repo references it via npm workspace.

---

## Implementation Order

Execute in this order to keep the repo green at each step:

### Phase 1: Storage (no behavior change)

1. Add migration `004_external_identity.sql`
2. Add `external_id: Option<String>` to `UserRow`, `OrganizationRow`, `CreateUserRow`
3. Add new storage methods (`get_user_by_external_id`, `get_organization_by_external_id`, `upsert_org_by_external_id`, `ensure_membership`)
4. Update in-memory backend to match
5. **Test:** `cargo test` — all existing tests pass, new methods tested

### Phase 2: Auth backend trait (no behavior change)

6. Create `auth/backend.rs` with `AuthBackend` trait
7. Create `auth/builtin.rs` implementing `AuthBackend` with existing logic from `middleware.rs`
8. Update `auth/mod.rs` exports
9. Refactor `AuthState` to hold `Arc<dyn AuthBackend>`
10. Update `extract_auth_user()` to use `backend.validate_token()` / `backend.validate_api_key()`
11. Update `main.rs` to create `BuiltinAuthBackend` and pass to `AuthState`
12. **Test:** `cargo test` — identical behavior, all tests pass

### Phase 3: Server entrypoint extraction (no behavior change)

13. Create `server.rs` with `ServerConfig` and `run()` function
14. Move bootstrap logic from `main.rs` → `server.rs`
15. Make `main.rs` thin (call `run()` with `BuiltinAuthBackend`)
16. Update `lib.rs` exports
17. **Test:** `cargo test` + `just start-dev` — server starts normally

### Phase 4: UI exports (no behavior change)

18. Export `AuthContextValue` and `AuthContext` from `auth-provider.tsx`
19. Export `OrgContextValue` and `OrgContext` from `org-provider.tsx`
20. Add `exports` field to `apps/ui/package.json`
21. **Test:** `npm run build` + `npm run lint` — no regressions

---

## Testing Strategy

All changes are refactors with zero behavior change. Verify with:

```bash
# Rust
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features

# UI
cd apps/ui && npm run lint && npm run build && npm test

# Integration
just start-dev  # Verify server starts, UI loads, auth works in all 3 modes
```

### New tests to add

| Test | Location | Validates |
|------|----------|-----------|
| `BuiltinAuthBackend::validate_token` | `auth/builtin.rs` | JWT validation via trait |
| `BuiltinAuthBackend::validate_api_key` | `auth/builtin.rs` | API key validation via trait |
| `get_user_by_external_id` | `storage/repositories.rs` | External ID lookup |
| `upsert_org_by_external_id` | `storage/repositories.rs` | Idempotent org upsert |
| `ensure_membership` | `storage/repositories.rs` | Idempotent membership |
| `run()` with BuiltinAuthBackend | `server.rs` | Server starts via `run()` |

---

## Non-Goals (SaaS repo handles these)

- PropelAuth integration code
- Stripe billing code
- SaaS-specific database tables (`subscriptions`, `monthly_usage`)
- SaaS-specific UI pages (billing, usage)
- SaaS auth provider (`propelauth-provider.tsx`)
- SaaS deployment configuration
