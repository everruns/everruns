# SaaS Repository Specification

> Private repository that wraps the open-source Everruns codebase with SaaS-specific functionality.
> Zero SaaS code in the OSS repo. OSS repo has no knowledge of this repo.

## Principles

- OSS repo is a **git submodule** pinned to release tags
- SaaS repo produces two deployable artifacts: `saas-server` binary and `saas-ui` Next.js app
- Both reuse 95%+ of OSS code — only auth, billing, and infra config are SaaS-specific
- OSS changes are minimal, general-purpose, and beneficial to all users (e.g., `AuthBackend` trait)

## Repository Structure

```
everruns-saas/
├── CLAUDE.md                         # SaaS-specific agent guidance
├── Cargo.toml                        # Rust workspace root
├── package.json                      # npm workspace root
├── justfile                          # SaaS dev commands
├── .github/
│   └── workflows/
│       ├── ci.yml                    # Build + test SaaS crates
│       └── deploy.yml                # Deploy to production
│
├── oss/                              # Git submodule → github.com/everruns/everruns
│   ├── crates/
│   │   ├── server/                   # OSS server library + binary
│   │   ├── worker/
│   │   ├── core/
│   │   └── ...
│   └── apps/
│       └── ui/                       # OSS UI (Next.js)
│
├── crates/
│   ├── saas-server/                  # SaaS server binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs              # Entrypoint — wires SaaS backends
│   │       ├── auth/
│   │       │   ├── mod.rs
│   │       │   ├── propelauth.rs    # PropelAuth AuthBackend impl
│   │       │   └── sync.rs          # User/org sync from PropelAuth → DB
│   │       ├── billing/
│   │       │   ├── mod.rs
│   │       │   ├── stripe.rs        # Stripe subscription management
│   │       │   ├── usage.rs         # Usage metering + limits
│   │       │   └── routes.rs        # Billing API endpoints
│   │       └── config.rs            # SaaS-specific env config
│   │
│   └── saas-common/                  # Shared SaaS types (if needed)
│       ├── Cargo.toml
│       └── src/lib.rs
│
├── apps/
│   └── saas-ui/                      # SaaS UI (Next.js)
│       ├── package.json
│       ├── next.config.js            # Aliases + transpile OSS UI
│       ├── tsconfig.json
│       └── src/
│           ├── app/
│           │   ├── layout.tsx        # Root layout — SaaS providers
│           │   ├── (auth)/
│           │   │   ├── login/page.tsx      # → PropelAuth hosted login
│           │   │   └── register/page.tsx   # → PropelAuth hosted signup
│           │   ├── (main)/
│           │   │   ├── layout.tsx          # Re-export OSS with SaaS guard
│           │   │   ├── billing/page.tsx    # SaaS-only: subscription management
│           │   │   └── usage/page.tsx      # SaaS-only: usage dashboard
│           │   └── [...oss]/               # Catch-all re-exports OSS pages
│           ├── providers/
│           │   ├── propelauth-provider.tsx  # Wraps @propelauth/react
│           │   └── propelauth-org-provider.tsx
│           └── components/
│               ├── billing/                # Stripe elements, plan cards
│               └── usage/                  # Usage meters, limit warnings
│
├── infra/                             # IaC (Pulumi/Terraform)
│   ├── production/
│   └── staging/
│
└── scripts/
    ├── update-oss.sh                  # Update submodule to latest release
    └── seed-saas.sh                   # Seed SaaS-specific data
```

## OSS Changes Required (One-Time)

Minimal, general-purpose changes that benefit all users:

### 1. AuthBackend Trait (~50 lines)

```rust
// oss/crates/server/src/auth/backend.rs

/// Pluggable authentication backend.
/// OSS ships BuiltinAuthBackend. External implementations (PropelAuth, Auth0,
/// Keycloak) can implement this trait without modifying OSS code.
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// Validate a Bearer token and return the authenticated user.
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError>;

    /// Validate an API key (prefixed `evr_`) and return the authenticated user.
    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError>;

    /// Optional: return additional routes (login, register, OAuth callbacks).
    /// Returns None if auth is handled externally (e.g., PropelAuth hosted UI).
    fn routes(&self) -> Option<Router> {
        None
    }
}
```

### 2. Server Library Extraction (~20 lines moved)

Split `crates/server/src/main.rs` into `lib.rs` + `main.rs`:

```rust
// oss/crates/server/src/lib.rs
pub mod api;
pub mod auth;
pub mod storage;
// ...

pub async fn run(config: ServerConfig, auth: impl AuthBackend) -> Result<()> { ... }
```

```rust
// oss/crates/server/src/main.rs (OSS standalone — unchanged behavior)
#[tokio::main]
async fn main() {
    let config = ServerConfig::from_env();
    let auth = BuiltinAuthBackend::new(&config).await;
    everruns_server::run(config, auth).await.unwrap();
}
```

### 3. UI Package Exports (~10 lines)

```jsonc
// oss/apps/ui/package.json — add exports field
{
  "name": "@everruns/ui",
  "exports": {
    "./providers/*": "./src/providers/*.tsx",
    "./components/*": "./src/components/*.tsx",
    "./hooks/*": "./src/hooks/*.ts",
    "./lib/*": "./src/lib/*.ts",
    "./app/*": "./src/app/*"
  }
}
```

### 4. AuthContext Type Export

```typescript
// oss/apps/ui/src/providers/auth-provider.tsx — export the interface
export interface AuthContextValue {
  config: AuthConfigResponse | undefined;
  user: UserInfoResponse | null;
  isAuthenticated: boolean;
  requiresAuth: boolean;
  isLoading: boolean;
}
```

## SaaS Server

### Entrypoint

```rust
// crates/saas-server/src/main.rs
use everruns_server::{ServerConfig, run_with};

mod auth;
mod billing;
mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server_config = ServerConfig::from_env();
    let saas_config = config::SaasConfig::from_env();

    let auth = auth::PropelAuthBackend::new(&saas_config).await?;

    let extra_routes = billing::routes(&saas_config);

    everruns_server::run_with(server_config, auth, extra_routes).await
}
```

### PropelAuth Backend

```rust
// crates/saas-server/src/auth/propelauth.rs
use everruns_server::auth::{AuthBackend, AuthUser, AuthError, OrgMembership};
use propelauth::auth::PropelAuth;

pub struct PropelAuthBackend {
    client: PropelAuth,
    db: StorageBackend,  // For user/org sync
}

#[async_trait]
impl AuthBackend for PropelAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        let pa_user = self.client
            .verify_access_token(token)
            .map_err(|_| AuthError::unauthorized("Invalid token"))?;

        // Sync user to local DB on first login (upsert)
        let user = self.sync_user(&pa_user).await?;

        // Map PropelAuth orgs → Everruns OrgMembership
        let orgs = pa_user.org_memberships.iter()
            .map(|o| self.map_org(o))
            .collect::<Result<Vec<_>>>()?;

        Ok(AuthUser {
            id: user.id,
            email: pa_user.email.clone(),
            name: format!("{} {}", pa_user.first_name, pa_user.last_name),
            roles: derive_roles(&pa_user),
            auth_method: AuthMethod::Jwt,
            organizations: orgs,
        })
    }

    async fn validate_api_key(&self, key: &str) -> Result<AuthUser, AuthError> {
        // API keys remain in Everruns DB (not PropelAuth).
        // Validate key hash → look up user → validate user still exists in PropelAuth.
        let key_row = self.db.get_api_key_by_hash(&hash_api_key(key)).await?
            .ok_or_else(|| AuthError::unauthorized("Invalid API key"))?;

        if key_row.is_expired() {
            return Err(AuthError::unauthorized("API key expired"));
        }

        let user = self.db.get_user(key_row.user_id).await?;
        Ok(AuthUser { /* ... */ })
    }

    fn routes(&self) -> Option<Router> {
        None  // Auth UI handled by PropelAuth hosted pages
    }
}
```

### User/Org Sync

PropelAuth is the source of truth for users and orgs. On every token validation:

1. **User upsert** — if user doesn't exist in Everruns DB, insert. Update email/name if changed.
2. **Org upsert** — if org doesn't exist, create with `public_id` derived from PropelAuth org ID.
3. **Membership sync** — update org memberships to match PropelAuth.

```rust
// crates/saas-server/src/auth/sync.rs

impl PropelAuthBackend {
    async fn sync_user(&self, pa_user: &PropelAuthUser) -> Result<UserRow> {
        let external_id = pa_user.user_id.to_string();
        match self.db.get_user_by_external_id(&external_id).await? {
            Some(user) => {
                // Update if email/name changed
                self.db.update_user_profile(user.id, &pa_user.email, &name).await?;
                Ok(user)
            }
            None => {
                self.db.create_user(CreateUserRow {
                    external_id: Some(external_id),
                    email: pa_user.email.clone(),
                    name: format!("{} {}", pa_user.first_name, pa_user.last_name),
                    password_hash: None,  // No password — PropelAuth manages auth
                }).await
            }
        }
    }

    async fn sync_orgs(&self, pa_user: &PropelAuthUser) -> Result<Vec<OrgMembership>> {
        let mut memberships = Vec::new();
        for pa_org in &pa_user.org_memberships {
            let org = self.db.upsert_org_by_external_id(
                &pa_org.org_id.to_string(),
                &pa_org.org_name,
            ).await?;
            self.db.ensure_membership(user_id, org.org_id).await?;
            memberships.push(OrgMembership { org_id: org.org_id, public_id: org.public_id, name: org.name });
        }
        Ok(memberships)
    }
}
```

### Billing Module

```rust
// crates/saas-server/src/billing/routes.rs

pub fn routes(config: &SaasConfig) -> Router {
    Router::new()
        .route("/v1/billing/subscription", get(get_subscription))
        .route("/v1/billing/portal", post(create_portal_session))
        .route("/v1/billing/usage", get(get_usage_summary))
        .route("/v1/webhooks/stripe", post(stripe_webhook))
}
```

Billing is org-scoped. Each org has one Stripe subscription. Usage limits enforced at the server layer before LLM calls.

### SaaS Config

```rust
// crates/saas-server/src/config.rs
pub struct SaasConfig {
    // PropelAuth
    pub propelauth_url: String,            // PROPELAUTH_URL
    pub propelauth_api_key: String,        // PROPELAUTH_API_KEY

    // Stripe
    pub stripe_secret_key: String,         // STRIPE_SECRET_KEY
    pub stripe_webhook_secret: String,     // STRIPE_WEBHOOK_SECRET
    pub stripe_price_id: String,           // STRIPE_PRICE_ID

    // Limits
    pub free_tier_monthly_tokens: u64,     // FREE_TIER_MONTHLY_TOKENS
}
```

## SaaS UI

### Provider Replacement

```tsx
// apps/saas-ui/src/providers/propelauth-provider.tsx
"use client";

import { AuthProvider as PAProvider, useAuthInfo } from "@propelauth/react";
import { AuthContext, type AuthContextValue } from "@everruns/ui/providers/auth-provider";

export function SaasAuthProvider({ children }: { children: React.ReactNode }) {
  return (
    <PAProvider authUrl={process.env.NEXT_PUBLIC_PROPELAUTH_URL!}>
      <AuthBridge>{children}</AuthBridge>
    </PAProvider>
  );
}

function AuthBridge({ children }: { children: React.ReactNode }) {
  const auth = useAuthInfo();

  const value: AuthContextValue = {
    config: {
      mode: "full",
      password_auth_enabled: false,
      oauth_providers: [],
      signup_enabled: true,
    },
    user: auth.user
      ? {
          id: auth.user.userId,
          email: auth.user.email,
          name: `${auth.user.firstName} ${auth.user.lastName}`,
          roles: [],
          avatar_url: auth.user.pictureUrl ?? null,
          organizations: auth.orgHelper?.getOrgs().map((o) => ({
            public_id: o.orgId,
            name: o.orgName,
          })) ?? [],
        }
      : null,
    isAuthenticated: auth.isLoggedIn,
    requiresAuth: true,
    isLoading: auth.loading,
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}
```

### Login Page (Redirect to PropelAuth)

```tsx
// apps/saas-ui/src/app/(auth)/login/page.tsx
"use client";

import { useEffect } from "react";
import { useRedirectToLogin } from "@propelauth/react";
import { Loader2 } from "lucide-react";

export default function LoginPage() {
  const { redirectToLoginPage } = useRedirectToLogin();
  useEffect(() => {
    redirectToLoginPage({ postLoginRedirectUrl: window.location.origin + "/dashboard" });
  }, [redirectToLoginPage]);
  return (
    <div className="flex h-screen items-center justify-center">
      <Loader2 className="h-6 w-6 animate-spin" />
    </div>
  );
}
```

### OSS Page Re-exports

Most pages are identical to OSS. Re-export them:

```tsx
// apps/saas-ui/src/app/(main)/agents/page.tsx
export { default } from "@everruns/ui/app/(main)/agents/page";
```

Or use Next.js rewrites in `next.config.js` to serve OSS pages without individual re-export files.

### SaaS-Only Pages

```
apps/saas-ui/src/app/(main)/
├── billing/page.tsx          # Subscription management (Stripe)
├── usage/page.tsx            # Token usage dashboard with limits
└── settings/
    └── plan/page.tsx         # Plan upgrade/downgrade
```

### Next.js Config

```js
// apps/saas-ui/next.config.js
const path = require("path");

module.exports = {
  transpilePackages: ["@everruns/ui"],
  webpack: (config) => {
    config.resolve.alias = {
      ...config.resolve.alias,
      "@oss": path.resolve(__dirname, "../../oss/apps/ui/src"),
    };
    return config;
  },
  env: {
    NEXT_PUBLIC_PROPELAUTH_URL: process.env.NEXT_PUBLIC_PROPELAUTH_URL,
    NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY: process.env.NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY,
  },
};
```

## Cargo Workspace

```toml
# everruns-saas/Cargo.toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.dependencies]
everruns-server = { path = "oss/crates/server" }
everruns-core = { path = "oss/crates/core" }
propelauth = "0.5"
stripe-rust = "0.25"
tokio = { version = "1.42", features = ["full"] }
axum = "0.8"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
tracing = "0.1"
async-trait = "0.1"
```

```toml
# crates/saas-server/Cargo.toml
[package]
name = "everruns-saas-server"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "everruns-saas-server"
path = "src/main.rs"

[dependencies]
everruns-server = { workspace = true }
everruns-core = { workspace = true }
propelauth = { workspace = true }
stripe-rust = { workspace = true }
tokio = { workspace = true }
axum = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
async-trait = { workspace = true }
```

## npm Workspace

```jsonc
// everruns-saas/package.json
{
  "private": true,
  "workspaces": [
    "oss/apps/ui",
    "apps/saas-ui"
  ]
}
```

```jsonc
// apps/saas-ui/package.json
{
  "name": "@everruns/saas-ui",
  "private": true,
  "dependencies": {
    "@everruns/ui": "workspace:*",
    "@propelauth/react": "^4.0",
    "@stripe/react-stripe-js": "^3.0",
    "@stripe/stripe-js": "^5.0",
    "next": "^16.0",
    "react": "^19.0",
    "react-dom": "^19.0"
  }
}
```

## Git Submodule Management

```bash
# Initial setup
git submodule add git@github.com:everruns/everruns.git oss
cd oss && git checkout v0.6.0 && cd ..
git add oss && git commit -m "chore: pin OSS submodule to v0.6.0"

# Update to new OSS release
cd oss && git fetch origin && git checkout v0.7.0 && cd ..
git add oss && git commit -m "chore: update OSS submodule to v0.7.0"

# Verify compatibility
just test  # Runs SaaS tests against updated OSS
```

## Dev Workflow

```bash
# justfile
start-saas:          # Start SaaS server + UI in dev mode
    cargo run -p everruns-saas-server &
    cd apps/saas-ui && npm run dev

test:                # Run all SaaS tests
    cargo test -p everruns-saas-server
    cd apps/saas-ui && npm test

update-oss:          # Update OSS submodule
    cd oss && git fetch origin && git checkout {{tag}}

pre-pr:              # SaaS pre-PR checks
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cd apps/saas-ui && npm run lint && npm run build
```

## Environment Variables

### SaaS Server

| Variable | Description |
|----------|-------------|
| `PROPELAUTH_URL` | PropelAuth instance URL |
| `PROPELAUTH_API_KEY` | PropelAuth backend API key |
| `STRIPE_SECRET_KEY` | Stripe secret key |
| `STRIPE_WEBHOOK_SECRET` | Stripe webhook signing secret |
| `STRIPE_PRICE_ID` | Default subscription price ID |
| `FREE_TIER_MONTHLY_TOKENS` | Token limit for free tier |
| All OSS `AUTH_*` vars | Ignored — PropelAuth replaces built-in auth |
| All other OSS vars | Passed through to `everruns-server` |

### SaaS UI

| Variable | Description |
|----------|-------------|
| `NEXT_PUBLIC_PROPELAUTH_URL` | PropelAuth frontend auth URL |
| `NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY` | Stripe publishable key |
| `NEXT_PUBLIC_API_URL` | Backend API URL (same as OSS) |

## Database Schema Additions

SaaS-specific tables, added via separate migration directory:

```sql
-- Users: add external_id column for PropelAuth user mapping
ALTER TABLE users ADD COLUMN external_id TEXT UNIQUE;

-- Organizations: add external_id for PropelAuth org mapping
ALTER TABLE organizations ADD COLUMN external_id TEXT UNIQUE;

-- Stripe subscriptions (SaaS-only table)
CREATE TABLE subscriptions (
    id BIGSERIAL PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    stripe_customer_id TEXT NOT NULL UNIQUE,
    stripe_subscription_id TEXT UNIQUE,
    plan TEXT NOT NULL DEFAULT 'free',  -- free, pro, enterprise
    status TEXT NOT NULL DEFAULT 'active',
    current_period_start TIMESTAMPTZ,
    current_period_end TIMESTAMPTZ,
    monthly_token_limit BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_subscriptions_org_id ON subscriptions(org_id);

-- Usage tracking (monthly aggregation for billing)
CREATE TABLE monthly_usage (
    id BIGSERIAL PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    month DATE NOT NULL,  -- first day of month
    total_input_tokens BIGINT NOT NULL DEFAULT 0,
    total_output_tokens BIGINT NOT NULL DEFAULT 0,
    total_tokens BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, month)
);
```

Migrations run from `crates/saas-server/migrations/` — separate from OSS migrations in `oss/crates/server/migrations/`.

## Deployment

```
┌─────────────────────────────────────────────────────┐
│  Production                                          │
│                                                      │
│  ┌──────────────┐   ┌───────────────────────────┐   │
│  │  saas-ui     │──▶│  saas-server               │   │
│  │  (Vercel /   │   │  (Fly.io / Railway / ECS)  │   │
│  │   CloudFront)│   │                             │   │
│  └──────────────┘   │  everruns-server (lib)      │   │
│                      │  + PropelAuth backend       │   │
│                      │  + Stripe billing           │   │
│                      │  + Worker (embedded)        │   │
│                      └──────────┬────────────────┘   │
│                                 │                     │
│  ┌──────────────┐   ┌──────────▼────────────────┐   │
│  │  PropelAuth   │   │  PostgreSQL               │   │
│  │  (hosted)     │   │  (Neon / RDS / Supabase)  │   │
│  └──────────────┘   └───────────────────────────┘   │
│                                                      │
│  ┌──────────────┐                                    │
│  │  Stripe       │                                    │
│  │  (hosted)     │                                    │
│  └──────────────┘                                    │
└─────────────────────────────────────────────────────┘
```

## What Ships Where

| Component | OSS (`everruns-server`) | SaaS (`everruns-saas-server`) |
|-----------|:-:|:-:|
| Core API (agents, sessions, chat) | ✅ | ✅ (via lib) |
| Durable execution engine | ✅ | ✅ (via lib) |
| Built-in auth (JWT, password, OAuth) | ✅ `BuiltinAuthBackend` | — |
| PropelAuth integration | — | ✅ `PropelAuthBackend` |
| Stripe billing | — | ✅ |
| Usage limits enforcement | — | ✅ |
| **UI: Core pages** | ✅ | ✅ (via `@everruns/ui`) |
| **UI: Login/register** | ✅ built-in forms | ✅ PropelAuth redirect |
| **UI: Billing pages** | — | ✅ |
| **UI: Auth providers** | ✅ `AuthProvider` | ✅ `SaasAuthProvider` |
