# Everruns SaaS Gap Analysis

Gap analysis of what's missing to operate Everruns as a multi-tenant SaaS product.

**Current state**: Self-hosted durable AI agent platform with multi-tenancy, auth, usage tracking, and PostgreSQL-backed execution. No billing, no email, no rate limiting, no cloud deployment manifests.

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Exists and SaaS-ready |
| ⚠️ | Partially exists, needs work |
| ❌ | Missing entirely |

---

## 1. Billing & Payments ❌

**Current state**: Zero billing code. No Stripe, no invoices, no plans.

**What exists**: `llm_generations` table tracks token usage per-generation with denormalized totals on sessions and agents. This is a solid foundation for metered billing.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Subscription plans | P0 | Define tiers (Free, Pro, Enterprise) with resource limits |
| Payment processor | P0 | Stripe integration for subscriptions + metered usage |
| Usage metering | P0 | Aggregate token usage into billable units per billing cycle |
| Plan enforcement | P0 | Reject requests when plan limits exceeded (sessions, tokens, storage) |
| Billing portal | P1 | Self-service plan management, payment methods, invoices |
| Usage dashboard | P1 | Per-org usage visualization with breakdown by agent/model |
| Trial management | P1 | Free trial with automatic conversion/expiration |
| Overage handling | P1 | Soft limits with warnings vs hard cutoffs |
| Invoice generation | P2 | Monthly invoices with line items |
| Proration | P2 | Mid-cycle plan changes |
| Tax handling | P2 | Stripe Tax or similar for VAT/sales tax |

### Suggested plan dimensions

- Max agents per org
- Max concurrent sessions
- Max tokens per month (input + output)
- Max file storage (MB)
- Max SQL databases per session
- Max scheduled tasks
- Model access (which LLM providers/models available)
- SSE connection limits
- API request rate limits

---

## 2. Rate Limiting ❌

**Current state**: No rate limiting. Threat model lists this as open threat (TM-AUTH-001, TM-TOOL-006, TM-DOS-005).

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| API rate limiter | P0 | Per-org, per-API-key request rate limits (tower middleware) |
| Login rate limiting | P0 | Per-IP rate limiting on auth endpoints to prevent brute force |
| LLM call rate limiting | P0 | Per-org token/request limits to prevent cost runaway |
| SSE connection limits | P1 | Max concurrent SSE connections per org |
| File upload rate limiting | P1 | Per-org upload bandwidth and count limits |
| Graduated limits | P2 | Different rate limits per subscription plan |

### Recommended approach

Use `tower::limit` or a Redis-backed sliding window limiter. Store plan-specific limits in org config. Return `429 Too Many Requests` with `Retry-After` header.

---

## 3. Email System ❌

**Current state**: `email_verified` column exists in users table but no email delivery. No SMTP, SendGrid, or SES integration.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Email delivery service | P0 | Integration with SendGrid, SES, or Resend |
| Email verification | P0 | Verify user email on registration |
| Password reset | P0 | Forgot password flow with secure token |
| Transactional emails | P1 | Welcome, usage alerts, plan changes, invoice receipts |
| Organization invitations | P1 | Email-based team invites |
| Usage alerts | P2 | Approaching plan limits, overage warnings |
| Email templates | P2 | Branded HTML templates |

---

## 4. RBAC & Permissions ⚠️

**Current state**: Organization membership exists. `roles` JSONB field in membership table but unused. All members have equal access. Spec explicitly defers roles to v2.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Role definitions | P0 | Owner, Admin, Member, Viewer roles per org |
| Permission enforcement | P0 | Middleware/extractors that check role before action |
| Role assignment | P1 | API + UI to assign/change member roles |
| Resource-level permissions | P2 | Per-agent or per-session access control |
| API key scoping | P2 | Enforce API key scopes (currently tracked but not enforced) |
| Custom roles | P3 | User-defined permission sets |

### Suggested role matrix

| Action | Owner | Admin | Member | Viewer |
|--------|-------|-------|--------|--------|
| Manage billing | ✅ | ❌ | ❌ | ❌ |
| Manage members | ✅ | ✅ | ❌ | ❌ |
| Create/edit agents | ✅ | ✅ | ✅ | ❌ |
| Create sessions | ✅ | ✅ | ✅ | ❌ |
| View sessions | ✅ | ✅ | ✅ | ✅ |
| Manage LLM providers | ✅ | ✅ | ❌ | ❌ |
| Manage API keys | ✅ | ✅ | Own only | ❌ |
| Delete org | ✅ | ❌ | ❌ | ❌ |

---

## 5. Quotas & Usage Enforcement ❌

**Current state**: Token usage tracked per-generation. No enforcement. No storage quotas. Threat model notes TM-FS-008 (no file storage quota) as open.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Token budget enforcement | P0 | Per-org monthly token limits, reject when exceeded |
| Storage quota enforcement | P0 | Per-session file storage limits, per-org aggregate |
| Concurrent session limits | P0 | Max active sessions per org |
| Agent count limits | P1 | Max agents per org per plan |
| SQL database limits | P1 | Already partially exists (10 per session) but not per-org |
| Schedule count limits | P1 | Max scheduled tasks per org |
| Quota check middleware | P1 | Centralized quota checking before resource creation |
| Usage aggregation jobs | P1 | Periodic rollup of usage into daily/monthly buckets |
| Quota reset on billing cycle | P2 | Reset monthly token counters aligned with subscription |

---

## 6. User Self-Service ⚠️

**Current state**: Login, registration, OAuth, API key management exist. Missing account lifecycle features.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Password reset flow | P0 | Email-based password reset with secure tokens |
| Email change | P1 | Change email with verification of new address |
| Account deletion | P1 | GDPR-compliant account deletion with data purge |
| Profile management | P1 | Update display name, avatar, preferences |
| Session management | P2 | View/revoke active sessions |
| 2FA/MFA | P2 | TOTP or WebAuthn second factor |
| Login history | P3 | View recent login activity |

---

## 7. Organization Management ⚠️

**Current state**: Orgs exist. Users can belong to multiple. No invites, no role management UI, no org settings.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Invitation system | P0 | Invite users by email with role assignment |
| Member management UI | P0 | View, add, remove, change roles |
| Org settings page | P1 | Name, logo, default LLM provider, billing info |
| Org creation by users | P1 | Self-service org creation (currently seed-only) |
| Org deletion | P1 | With cascade cleanup of all resources |
| Transfer ownership | P2 | Transfer org owner role |
| SSO/SAML | P3 | Enterprise SSO per org |

---

## 8. Webhooks & Integrations ❌

**Current state**: No webhook infrastructure. SSE streaming exists for real-time events but only for connected clients.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Webhook registration | P1 | Per-org webhook URL configuration |
| Event delivery | P1 | Reliable delivery with retries and backoff |
| Webhook signing | P1 | HMAC signatures for payload verification |
| Event types | P1 | session.completed, turn.failed, usage.threshold, etc. |
| Delivery logs | P2 | Webhook delivery history with status |
| Webhook testing | P2 | Test endpoint to send sample payloads |

---

## 9. Data Retention & Cleanup ⚠️

**Current state**: `expires_at` on API keys and refresh tokens. No general data retention policy or automated cleanup.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Session TTL | P1 | Auto-archive/delete idle sessions after configurable period |
| Event retention | P1 | Prune old events beyond retention window |
| File cleanup | P1 | Remove orphaned files, enforce storage quotas |
| Account data export | P1 | GDPR data portability (export all user data) |
| Account data deletion | P1 | GDPR right to erasure |
| Retention policies per org | P2 | Configurable retention per plan tier |
| Archival storage | P2 | Move old data to cold storage |

---

## 10. Object Storage ⚠️

**Current state**: Images stored as BYTEA in PostgreSQL (max 100MB per file). Session files in PostgreSQL BYTEA. This won't scale.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| S3-compatible storage | P1 | Move images and large files to S3/R2/MinIO |
| Presigned URLs | P1 | Direct upload/download without proxying through server |
| CDN integration | P2 | Serve static assets and images via CDN |
| Storage tiering | P3 | Hot/cold storage based on access patterns |

---

## 11. Cloud Deployment & Infrastructure ⚠️

**Current state**: Docker images build via CI. Docker Compose for local/example deployment. No Kubernetes, no Terraform, no cloud-native deployment.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Kubernetes manifests | P0 | Deployments, Services, ConfigMaps for server + workers |
| Helm chart | P1 | Parameterized deployment for different environments |
| Horizontal pod autoscaling | P1 | Scale workers based on queue depth |
| Database migrations job | P1 | K8s Job for migrations (vs on-startup) |
| Readiness/liveness probes | P1 | `/health` exists but needs `/ready` (DB connected, workers available) |
| Terraform/Pulumi modules | P2 | Infrastructure as code for PostgreSQL, networking, secrets |
| Multi-region support | P3 | Data residency and latency optimization |

---

## 12. Security Hardening for SaaS ⚠️

**Current state**: Threat model is comprehensive. Several items marked OPEN.

### Gaps (from threat model OPEN items)

| Gap | Priority | Description |
|-----|----------|-------------|
| gRPC authentication | P0 | TM-DURABLE-002: Worker↔Server gRPC is unauthenticated |
| Security headers | P0 | TM-WEB-004/005: Missing CSP, X-Frame-Options, HSTS |
| CORS configuration | P0 | Proper CORS for SaaS domain(s) |
| DDoS protection | P1 | CloudFlare or similar edge protection |
| WAF | P1 | Web Application Firewall for API endpoints |
| Vulnerability scanning | P1 | Automated dependency scanning (cargo-audit in CI) |
| Penetration testing | P2 | Third-party security audit before launch |
| SOC 2 compliance | P3 | Audit controls, logging, access reviews |

---

## 13. Observability for SaaS Operations ⚠️

**Current state**: OpenTelemetry tracing, Jaeger for development. No production monitoring stack.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Metrics collection | P0 | Prometheus metrics for request latency, error rates, queue depth |
| Alerting | P0 | PagerDuty/OpsGenie alerts on error spikes, queue backup, DB issues |
| Log aggregation | P1 | Structured logging to Datadog/Grafana Loki |
| Status page | P1 | Public status page (Statuspage.io or similar) |
| SLO tracking | P1 | Define and track availability/latency SLOs |
| Customer-facing metrics | P2 | Per-org usage dashboards |
| Cost attribution | P2 | Track infrastructure cost per org |

---

## 14. Onboarding & Growth ❌

**Current state**: No onboarding flow, no templates, no getting-started experience.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Onboarding wizard | P1 | Guide new users through first agent creation |
| Agent templates | P1 | Pre-built agents for common use cases |
| Interactive tutorial | P1 | Step-by-step walkthrough of key features |
| Sample data / demo mode | P2 | Pre-populated org with example sessions |
| API quickstart | P2 | Copy-paste code snippets for common languages |
| Documentation site | ✅ | Already exists (Astro Starlight) |

---

## 15. Legal & Compliance ❌

**Current state**: Privacy-conscious design (OTel content recording disabled by default). No formal compliance.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Terms of Service | P0 | Legal ToS for SaaS users |
| Privacy Policy | P0 | GDPR-compliant privacy policy |
| Cookie consent | P1 | Cookie banner for web UI |
| Data Processing Agreement | P1 | DPA for enterprise customers |
| GDPR data export | P1 | Export all user data on request |
| GDPR data deletion | P1 | Complete data erasure on request |
| Audit logging | P1 | Track who did what and when for compliance |
| SOC 2 readiness | P3 | Access controls, encryption, monitoring |
| HIPAA considerations | P3 | If healthcare use cases targeted |

---

## 16. Landing Page & Marketing ❌

**Current state**: Documentation site exists. No marketing site, pricing page, or public signup.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| Landing page | P0 | Product marketing page with value proposition |
| Pricing page | P0 | Plan comparison with signup CTAs |
| Public signup flow | P0 | Self-service registration → org creation → onboarding |
| Blog | P2 | Content marketing, release announcements |
| Changelog | ⚠️ | CHANGELOG.md exists; needs public-facing version |
| API documentation portal | ⚠️ | Swagger UI exists; needs polished developer portal |

---

## 17. Notification System ❌

**Current state**: No in-app or out-of-app notifications beyond SSE streaming.

### Gaps

| Gap | Priority | Description |
|-----|----------|-------------|
| In-app notifications | P1 | Notification center in UI for events, alerts |
| Email notifications | P1 | Session failures, usage alerts, team invites |
| Notification preferences | P2 | Per-user opt-in/out for notification types |
| Slack/Discord integration | P3 | Forward notifications to team channels |

---

## Priority Summary

### P0 — Must have before launch

1. **Billing**: Stripe integration, plans, enforcement
2. **Rate limiting**: API + auth + LLM call limits
3. **Email**: Delivery service, verification, password reset
4. **Security headers**: CSP, HSTS, X-Frame-Options, CORS
5. **gRPC auth**: mTLS or bearer tokens for worker communication
6. **Kubernetes manifests**: Production deployment
7. **Legal**: Terms of Service, Privacy Policy
8. **Landing/pricing page**: Public-facing marketing
9. **Token budget enforcement**: Prevent cost runaway per org
10. **RBAC**: At minimum Owner/Admin/Member roles

### P1 — Required for production readiness

1. Organization invitations and member management
2. Self-service account lifecycle (password reset, deletion)
3. Webhooks for event delivery
4. S3-compatible object storage
5. Monitoring, alerting, status page
6. Usage dashboard per org
7. Data retention policies
8. GDPR compliance (export, deletion)
9. Onboarding experience
10. Helm chart and autoscaling

### P2 — Growth and polish

1. 2FA/MFA
2. Proration and tax handling
3. CDN for static assets
4. Infrastructure as code
5. API quickstart guides
6. Notification preferences
7. Webhook delivery logs

### P3 — Enterprise / future

1. SSO/SAML per org
2. Custom roles
3. SOC 2 / HIPAA
4. Multi-region
5. Slack/Discord integrations
6. Storage tiering

---

## Architectural Recommendations

### 1. Add a `plans` table and `org_plan` relationship
```
plans: id, name, slug, limits_json, price_monthly, price_yearly, stripe_price_id
org_subscriptions: id, org_id, plan_id, stripe_subscription_id, status, current_period_start, current_period_end
```

### 2. Centralized quota middleware
Add an Axum middleware layer that resolves org → plan → limits and checks usage before allowing resource creation. Return `402 Payment Required` or `429 Too Many Requests` with detail.

### 3. Email as a core crate
Add `crates/email/` with trait-based email sending. Implementations for SES/SendGrid/SMTP. Queue emails via the durable engine for reliability.

### 4. Redis for rate limiting and caching
Add Redis as infrastructure dependency for:
- Sliding window rate limiting
- Session cache / hot data
- Pub/sub for multi-instance SSE fanout

### 5. Migrate large blobs to S3
Move `images.data` and `session_files.content` to S3-compatible storage. Store only metadata + S3 key in PostgreSQL. Use presigned URLs for direct client upload/download.

### 6. Background billing aggregation
Use the existing durable engine to run periodic (hourly/daily) usage aggregation jobs that feed the billing system.

---

## What's Already SaaS-Ready

- ✅ Multi-tenancy with org-scoped resource isolation
- ✅ Authentication (JWT, API keys, OAuth with Google/GitHub)
- ✅ Token usage tracking per-generation
- ✅ Envelope encryption for secrets
- ✅ Durable execution engine (PostgreSQL-backed, no external deps)
- ✅ Event sourcing architecture (audit trail foundation)
- ✅ OpenAPI spec and Swagger UI
- ✅ Docker images with CI/CD
- ✅ OpenTelemetry instrumentation
- ✅ Threat model with categorized risks
- ✅ Dual-ID pattern (internal UUID + public prefixed ID)
- ✅ Scheduled tasks with cron
- ✅ Documentation site
