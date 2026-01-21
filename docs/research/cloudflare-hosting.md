# Cloudflare Hosting Research

## Executive Summary

**Can Everruns run on Cloudflare?** Partially yes, with significant caveats.

| Component | Cloudflare Option | Feasibility |
|-----------|-------------------|-------------|
| Control Plane (Rust) | Containers (beta) | Possible |
| Worker (Rust) | Containers (beta) | Possible |
| PostgreSQL | External via Hyperdrive | Yes, but external |
| UI (Next.js) | Pages | Yes |
| Docs (Astro) | Pages | Already deployed |

**Key Blockers:**
- D1 (Cloudflare's DB) is SQLite-based, not PostgreSQL - cannot use it
- Workers (WASM) don't support Tokio - cannot run axum/tonic
- Must use Containers (beta) for native Rust binaries
- PostgreSQL NOTIFY/LISTEN may have issues through Hyperdrive

## Everruns Architecture Requirements

From `specs/architecture.md` and `specs/durable-execution-engine.md`:

1. **Control Plane**: Rust (axum HTTP on :9000, tonic gRPC on :9001)
2. **Worker**: Rust durable worker with gRPC client
3. **Database**: PostgreSQL 17 with:
   - UUID v7 custom function
   - `SELECT FOR UPDATE SKIP LOCKED` for task claiming
   - `NOTIFY/LISTEN` for push-based task notifications
   - Multiple tables for durable workflow engine
4. **UI**: Next.js on :9100
5. **Docs**: Astro Starlight

## Cloudflare Hosting Options Analysis

### Option 1: Workers (WASM) - NOT SUITABLE

Cloudflare Workers compile Rust to WebAssembly and run in V8 isolates.

**Blockers:**
- **No Tokio support** - axum and tonic require Tokio async runtime
- **128MB memory limit** - may be insufficient for complex agent workloads
- **10MB bundle size** - unoptimized Rust WASM can exceed this
- **No threading** - single-threaded execution only
- **No native gRPC** - Workers don't support gRPC directly

**Limits:**
- Free: 10ms CPU time, 3MB bundle, 100K requests/day
- Paid: 30s-5min CPU time, 10MB bundle

**Conclusion:** Cannot run Everruns control plane or worker on Workers (WASM).

### Option 2: Cloudflare Containers (Beta) - POSSIBLE

New container platform (public beta since June 2025) that runs Docker images globally.

**Advantages:**
- Run native Rust binaries (no WASM conversion)
- Scale-to-zero billing
- 300+ edge locations
- Full Docker compatibility

**Instance Types:**
| Type | RAM | vCPU | Use Case |
|------|-----|------|----------|
| dev | 256MB | 1/16 | Development |
| basic | 1GB | 1/4 | Small workloads |
| standard | 4GB | 1/2 | Production |

**Limits (Beta):**
- Max 40GB RAM, 40 vCPU per account
- Requires Workers Paid plan ($5/month)

**Pricing (after included allowances):**
- CPU: $0.00002 per vCPU-second (active usage only)
- Memory: Based on provisioned resources
- Egress: $0.025-0.050/GB (1TB included)

**Included in $5/month base:**
- 25 GB-hours RAM
- 375 vCPU-minutes
- 200 GB-hours disk

**Risks:**
- Beta product - may have stability issues
- Cold start latency
- Limited instance sizes for heavy workloads

### Option 3: Cloudflare D1 - NOT SUITABLE

D1 is Cloudflare's serverless database, but it's SQLite-based.

**Blockers:**
- **SQLite, not PostgreSQL** - incompatible with Everruns schema
- **No NOTIFY/LISTEN** - critical for durable engine notifications
- **Single-threaded** - ~1000 queries/sec max (1ms queries)
- **10GB max per database**
- **No UUID v7 support**

**Conclusion:** Cannot use D1 for Everruns.

### Option 4: Hyperdrive (Connection Pooling) - POSSIBLE

Hyperdrive provides connection pooling and query caching for external PostgreSQL databases.

**Advantages:**
- Free on Workers plans
- Reduces connection latency (96% improvement claimed)
- Supports PostgreSQL and MySQL
- Works with Neon, Supabase, AWS RDS, CockroachDB, etc.

**Considerations:**
- **Transaction mode pooling** - connections returned after each transaction
- **NOTIFY/LISTEN concern** - may not work reliably through pooler
- Need external PostgreSQL provider

**Conclusion:** Use Hyperdrive for connection pooling to external PostgreSQL.

### Option 5: Pages - YES

Cloudflare Pages can host Next.js and Astro applications.

**For UI (Next.js):**
- Full Next.js support with edge functions
- 500 builds/month free, 5000 paid
- Unlimited bandwidth

**For Docs (Astro):**
- Already deployed at docs.everruns.com
- Works well

## Database Options (External)

Since D1 cannot be used, external PostgreSQL is required:

| Provider | Free Tier | Paid Starting | Notes |
|----------|-----------|---------------|-------|
| Neon | 0.5GB, 1 project | $19/month | Serverless, autoscaling |
| Supabase | 500MB, 2 projects | $25/month | Postgres + Auth + Storage |
| PlanetScale | N/A (MySQL) | - | Not PostgreSQL |
| AWS RDS | N/A | ~$15/month (t4g.micro) | Self-managed |
| DigitalOcean | N/A | $15/month | Managed Postgres |

**Recommendation:** Neon or Supabase for serverless compatibility.

## Cost Estimate

### Scenario 1: Development/Testing
- Cloudflare Containers (dev instance): $5/month base
- Neon Free tier: $0
- Pages: $0
- **Total: ~$5/month**

### Scenario 2: Small Production (100 agents, 1000 sessions/month)
- Cloudflare Containers (basic instance): ~$15-25/month
- Neon Launch tier: $19/month
- Pages: $0
- Hyperdrive: Included
- **Total: ~$35-45/month**

### Scenario 3: Medium Production (1000 agents, 10K sessions/month)
- Cloudflare Containers (standard instances x2): ~$50-100/month
- Neon Scale tier: ~$69/month
- Pages: $0
- **Total: ~$120-170/month**

### Scenario 4: Large Production
- Cloudflare Containers: $200-500/month
- PostgreSQL (dedicated): $100-300/month
- **Total: ~$300-800/month**

## Recommended Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Cloudflare Edge                           │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌──────────────────────────────────┐ │
│  │   Pages (UI)    │  │   Containers (Control Plane)     │ │
│  │   Next.js       │  │   Rust binary (axum + tonic)     │ │
│  │   :9100         │  │   :9000 HTTP, :9001 gRPC         │ │
│  └────────┬────────┘  └──────────────┬───────────────────┘ │
│           │                          │                      │
│           │           ┌──────────────┴───────────────────┐ │
│           │           │   Containers (Worker)            │ │
│           │           │   Rust binary (durable worker)   │ │
│           │           └──────────────┬───────────────────┘ │
│           │                          │                      │
│           │           ┌──────────────┴───────────────────┐ │
│           │           │   Hyperdrive (Connection Pool)   │ │
│           │           └──────────────┬───────────────────┘ │
└───────────│──────────────────────────│──────────────────────┘
            │                          │
            │                          ▼
            │           ┌──────────────────────────────────┐
            │           │   External PostgreSQL            │
            │           │   (Neon / Supabase / AWS RDS)    │
            │           └──────────────────────────────────┘
            │
            └──────► API calls to Control Plane containers
```

## Implementation Challenges

### Challenge 1: PostgreSQL NOTIFY/LISTEN

The durable execution engine uses `NOTIFY/LISTEN` for push-based task notifications:
- Control-plane listens via `PgListener`
- Pushes to workers via gRPC streaming

**Problem:** Hyperdrive uses transaction-mode connection pooling, which may not maintain persistent connections needed for `LISTEN`.

**Solutions:**
1. Keep a dedicated non-pooled connection for LISTEN
2. Fall back to polling (10s interval, already supported)
3. Use Cloudflare Queues for notifications instead

### Challenge 2: gRPC Between Services

Workers (both control plane and worker) communicate via gRPC.

**Problem:** Containers need to discover each other's addresses.

**Solutions:**
1. Use Cloudflare's internal networking (service bindings may work)
2. Use fixed container names/addresses
3. Deploy as single combined binary (not recommended)

### Challenge 3: Cold Starts

Containers scale to zero for cost savings but have cold start latency.

**Mitigations:**
1. Use minimum instances setting
2. Accept latency for initial requests
3. Health check probes to keep warm

### Challenge 4: Beta Stability

Containers are in public beta (as of 2025).

**Mitigations:**
1. Have fallback deployment option (Fly.io, Railway)
2. Monitor for issues
3. Wait for GA if risk-averse

## Comparison with Alternatives

| Platform | Monthly Cost (Small) | PostgreSQL | Rust Support | Cold Starts |
|----------|---------------------|------------|--------------|-------------|
| Cloudflare Containers | $35-45 | External | Native | Yes |
| Fly.io | $20-40 | Managed ($15+) | Native | ~300ms |
| Railway | $20-40 | Built-in | Native | Yes |
| Render | $25-50 | Built-in ($7+) | Native | ~5s |
| AWS Fargate + RDS | $50-100 | Managed | Native | No |
| DigitalOcean App Platform | $30-50 | Built-in ($15+) | Native | No |

## Recommendation

**For Everruns on Cloudflare:**

1. **Short-term (now):** Wait for Containers to exit beta OR use alternative (Fly.io/Railway recommended)

2. **Medium-term (when Containers GA):**
   - Deploy control plane and worker as Cloudflare Containers
   - Use Neon for PostgreSQL (via Hyperdrive)
   - Deploy UI on Pages
   - Test NOTIFY/LISTEN behavior; implement polling fallback

3. **What works today:**
   - Documentation on Cloudflare Pages (already deployed)
   - UI could move to Cloudflare Pages (static + API calls)

**Not recommended:**
- Attempting to run Everruns on Workers (WASM) - architectural incompatibility
- Using D1 for database - not PostgreSQL compatible

## Sources

- [Cloudflare Workers Rust Support](https://developers.cloudflare.com/workers/languages/rust/)
- [Cloudflare Workers Limits](https://developers.cloudflare.com/workers/platform/limits/)
- [Cloudflare D1 Overview](https://developers.cloudflare.com/d1/)
- [Cloudflare D1 Limits](https://developers.cloudflare.com/d1/platform/limits/)
- [Cloudflare Hyperdrive](https://developers.cloudflare.com/hyperdrive/)
- [Cloudflare Hyperdrive Pricing](https://developers.cloudflare.com/hyperdrive/platform/pricing/)
- [Cloudflare Durable Objects Pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/)
- [Cloudflare Workflows Pricing](https://developers.cloudflare.com/workflows/reference/pricing/)
- [Cloudflare Containers Pricing](https://developers.cloudflare.com/containers/pricing/)
- [Cloudflare Containers Announcement](https://blog.cloudflare.com/cloudflare-containers-coming-2025/)
- [Cloudflare Containers Beta Launch](https://blog.cloudflare.com/containers-are-available-in-public-beta-for-simple-global-and-programmable/)
- [Cloudflare Workers Pricing](https://developers.cloudflare.com/workers/platform/pricing/)
