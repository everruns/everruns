---
title: Hosting Cost Comparison
description: Cheapest options to host Everruns on AWS, Azure, and GCP
---

# Everruns Hosting Cost Comparison (2025)

## Infrastructure Requirements

Based on `specs/architecture.md` and `local/docker-compose.yml`:

| Component | Resource Needs | Notes |
|-----------|----------------|-------|
| **PostgreSQL 17** | 1GB+ RAM, 20GB+ storage | Required for persistence |
| **Control-plane** | 512MB-1GB RAM | HTTP (9000) + gRPC (9001) |
| **Worker** | 512MB-1GB RAM | Single worker sufficient |
| **UI** | Static files or SSR | Can run on same host |
| **Jaeger** (optional) | 256MB RAM | Tracing, not required |

**Minimum total**: 2GB RAM for all-in-one deployment

---

## AWS (Preferred)

### Option 1: Single EC2 Instance (Cheapest Production-Ready)

Run PostgreSQL + Control-plane + Worker + UI on one instance.

| Resource | Price | Notes |
|----------|-------|-------|
| **t4g.small** | **FREE until Dec 2026** | 2 vCPU, 2GB RAM, ARM64 |
| **t4g.micro** | ~$6/mo | 2 vCPU, 1GB RAM (too small) |
| **EBS gp3 30GB** | ~$2.40/mo | Storage |
| **Total** | **~$2.40/mo** | While free trial lasts |
| **After trial** | **~$14/mo** | t4g.small + storage |

**Pros**: Simplest, cheapest, single point of management
**Cons**: No managed database, need to backup PostgreSQL yourself

### Option 2: Lightsail All-in-One

| Plan | Price | Specs |
|------|-------|-------|
| **$10/mo** | $10/mo | 1GB RAM, 2 vCPU, 40GB SSD |
| **$20/mo** | $20/mo | 2GB RAM, 2 vCPU, 60GB SSD |

**Recommendation**: $20/mo plan for comfort (2GB RAM)

**Pros**: Predictable pricing, includes bandwidth
**Cons**: Less flexible than EC2

### Option 3: EC2 + Managed RDS

| Resource | Price | Notes |
|----------|-------|-------|
| **t4g.small EC2** | FREE / ~$12/mo | For control-plane + worker + UI |
| **db.t4g.micro RDS** | FREE tier / ~$22/mo | 750h/mo free for 12 months |
| **gp2 20GB** | Included in free tier | Must use gp2, not gp3 |
| **Total (free tier)** | **~$2.40/mo** | Storage only |
| **Total (post-trial)** | **~$36/mo** | After free tiers expire |

**Free tier note**: As of July 2025, new AWS accounts get $100 in credits instead of traditional free tier.

### Option 4: EC2 + Serverless Database

| Resource | Price | Notes |
|----------|-------|-------|
| **t4g.small EC2** | FREE / ~$12/mo | Compute |
| **Neon (free)** | $0/mo | 0.5GB storage, 100 CU-hours |
| **Supabase (free)** | $0/mo | 500MB, pauses after 1 week idle |
| **Total** | **~$0-12/mo** | |

**⚠️ Caveats**:
- Neon: 0.5GB storage may be limiting
- Supabase: Projects pause after 1 week inactivity (not suitable for production)
- Both require internet egress from EC2 to reach database

### Option 5: ECS Fargate Spot

| Resource | Price | Notes |
|----------|-------|-------|
| **Fargate Spot 0.5 vCPU / 1GB** | ~$10/mo | 70% discount, interruptible |
| **RDS db.t4g.micro** | FREE / ~$22/mo | |
| **Total** | **~$10-32/mo** | |

**Pros**: Fully managed containers, auto-restart
**Cons**: Spot can be interrupted (2 min warning)

### Option 6: App Runner

| Resource | Price | Notes |
|----------|-------|-------|
| **App Runner (1 vCPU, 2GB)** | ~$40-56/mo | No scale-to-zero |
| **RDS** | ~$22/mo | |
| **Total** | **~$62-78/mo** | Too expensive |

**Not recommended** - significantly more expensive than EC2/Lightsail.

---

## Azure

### Option 1: VM + Managed PostgreSQL

| Resource | Price | Notes |
|----------|-------|-------|
| **B1s VM** | ~$5-7/mo | 1 vCPU, 1GB RAM |
| **B1ms VM** | ~$12/mo | 1 vCPU, 2GB RAM |
| **PostgreSQL Flexible B1ms** | ~$12.41/mo | Free 750h/mo for 12 months |
| **Storage 32GB** | ~$4/mo | |
| **Total (free tier)** | **~$16-21/mo** | |
| **Total (post-trial)** | **~$28-33/mo** | |

### Option 2: All-in-One VM

| Resource | Price | Notes |
|----------|-------|-------|
| **B2s VM** | ~$30/mo | 2 vCPU, 4GB RAM |
| **Managed disk 32GB** | ~$2/mo | |
| **Total** | **~$32/mo** | |

---

## GCP

### Option 1: Compute Engine + Cloud SQL

| Resource | Price | Notes |
|----------|-------|-------|
| **e2-micro** | ~$6-7/mo | 0.25 vCPU, 1GB (free tier available) |
| **e2-small** | ~$13/mo | 0.5 vCPU, 2GB |
| **Cloud SQL (smallest)** | ~$30/mo | 1 vCPU, 3.75GB minimum |
| **Total** | **~$36-43/mo** | |

**Not recommended** - Cloud SQL minimum is expensive.

### Option 2: Cloud Run + External DB

| Resource | Price | Notes |
|----------|-------|-------|
| **Cloud Run** | Pay-per-request | Scales to zero |
| **Neon/Supabase** | $0/mo | External managed DB |
| **Total** | **~$5-15/mo** | Depending on traffic |

**Note**: Cloud Run scales to zero, great for low-traffic. Need external PostgreSQL.

---

## Comparison Summary

| Option | Monthly Cost | Managed DB | Complexity |
|--------|-------------|------------|------------|
| **AWS t4g.small (free trial)** | ~$2.40 | No | Low |
| **AWS Lightsail $10** | $10 | No | Low |
| **AWS Lightsail $20** | $20 | No | Low |
| **AWS t4g.small + Neon** | ~$12 | Yes (serverless) | Medium |
| **AWS EC2 + RDS (free tier)** | ~$2.40 | Yes | Medium |
| **AWS EC2 + RDS (paid)** | ~$36 | Yes | Medium |
| **Azure B1ms + PostgreSQL** | ~$28-33 | Yes | Medium |
| **GCP e2-small + Cloud SQL** | ~$43+ | Yes | Medium |

---

## Recommendation: AWS

### Cheapest Development/PoC (~$2-10/mo)

```
┌─────────────────────────────────────┐
│  AWS t4g.small EC2 (FREE until 2026)│
│  ─────────────────────────────────  │
│  • PostgreSQL 17 (self-hosted)      │
│  • Control-plane                    │
│  • Worker                           │
│  • UI (Next.js)                     │
│  • 30GB EBS gp3 (~$2.40/mo)         │
└─────────────────────────────────────┘
```

**Setup**:
```bash
# On Ubuntu 24.04 ARM64 (t4g)
sudo apt update
sudo apt install -y postgresql-17

# Build Everruns (ARM64)
cargo build --release

# Run all components
./target/release/everruns-control-plane &
./target/release/everruns-worker &
cd apps/ui && npm run build && npm start &
```

### Cheapest Production (~$20-36/mo)

**Option A**: Lightsail $20/mo (simpler)
```
┌─────────────────────────────────────┐
│  AWS Lightsail $20/mo               │
│  ─────────────────────────────────  │
│  • 2GB RAM, 2 vCPU, 60GB SSD        │
│  • PostgreSQL 17 (self-hosted)      │
│  • Daily snapshots ($0.05/GB)       │
│  • Control-plane + Worker + UI      │
└─────────────────────────────────────┘
```

**Option B**: EC2 + RDS (managed DB)
```
┌────────────────────┐    ┌────────────────────┐
│  t4g.small EC2     │    │  RDS db.t4g.micro  │
│  ~$12/mo           │───▶│  ~$22/mo           │
│  ────────────────  │    │  ────────────────  │
│  • Control-plane   │    │  • PostgreSQL 17   │
│  • Worker          │    │  • 20GB gp2        │
│  • UI              │    │  • Auto backups    │
└────────────────────┘    └────────────────────┘
```

---

## Cost Optimization Tips

1. **Use ARM64 (Graviton)**: t4g instances are ~20% cheaper than t3
2. **Reserved Instances**: 1-year commitment saves ~30-40%
3. **Spot for Worker**: Worker is stateless, can use Spot instances
4. **Right-size storage**: Start with 20GB, scale as needed
5. **Single AZ**: Multi-AZ doubles cost, skip for non-critical workloads

---

## Architecture Diagram (Recommended AWS Setup)

```
┌─────────────────────────────────────────────────────────────┐
│                    AWS VPC (us-east-1)                      │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │           EC2 t4g.small (or Lightsail)              │   │
│  │                                                     │   │
│  │  ┌───────────────┐  ┌───────────────┐              │   │
│  │  │ Control-plane │  │    Worker     │              │   │
│  │  │   :9000 HTTP  │◀─│  (gRPC client)│              │   │
│  │  │   :9001 gRPC  │  │               │              │   │
│  │  └───────┬───────┘  └───────────────┘              │   │
│  │          │                                          │   │
│  │          ▼                                          │   │
│  │  ┌───────────────┐  ┌───────────────┐              │   │
│  │  │ PostgreSQL 17 │  │   Next.js UI  │              │   │
│  │  │    :5432      │  │    :9100      │              │   │
│  │  └───────────────┘  └───────────────┘              │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    Internet (Elastic IP)
```

---

## Sources

- [AWS EC2 Pricing](https://aws.amazon.com/ec2/pricing/on-demand/)
- [AWS RDS PostgreSQL Pricing](https://aws.amazon.com/rds/postgresql/pricing/)
- [AWS Lightsail Pricing](https://aws.amazon.com/lightsail/pricing/)
- [AWS Fargate Pricing](https://aws.amazon.com/fargate/pricing/)
- [Azure PostgreSQL Flexible Server Pricing](https://azure.microsoft.com/en-us/pricing/details/postgresql/flexible-server/)
- [GCP Cloud SQL Pricing](https://cloud.google.com/sql/docs/postgres/pricing)
- [Neon Pricing](https://neon.com/pricing)
- [Supabase Pricing](https://supabase.com/pricing)
