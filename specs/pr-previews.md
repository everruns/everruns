# PR Preview Environments

## Abstract

This specification defines the PR preview environment system that automatically deploys isolated instances of the Everruns application for each pull request, enabling reviewers to test changes in a production-like environment before merging.

## Requirements

### Functional Requirements

1. **Automatic Deployment**: When a PR is opened or updated, deploy a complete preview environment
2. **Isolation**: Each PR gets its own isolated environment (separate database, services)
3. **Unique URLs**: Each preview has a unique, accessible URL
4. **Automatic Cleanup**: Environments are destroyed when PRs are merged or closed

### Non-Functional Requirements

1. **Cost Efficiency**: Scale-to-zero when idle, only pay for active usage
2. **Fast Deployment**: Preview should be available within minutes of push
3. **Minimal Maintenance**: No custom GitHub Actions workflow required

## Implementation: Railway Native GitHub Integration

Uses Railway's native GitHub integration with PR environments. Railway builds from source on each push - no Docker image coordination needed.

Based on `examples/docker-compose-full.yaml` but simplified for previews (1 worker instead of 3).

### Service Architecture

```
PR Preview Environment (mirrors docker-compose-full.yaml)
├── postgres      → Railway PostgreSQL template
├── api           → control-plane (HTTP :9000 + gRPC :9001)
├── worker        → 1 worker instance (connects to api:9001)
└── ui            → Next.js dashboard
```

### Setup Steps

#### Step 1: Create Railway Project
1. Go to https://railway.app/new
2. Select **Empty Project**
3. Name it `everruns-previews`

#### Step 2: Add PostgreSQL
1. Click **+ New** → **Database** → **Add PostgreSQL**
2. Railway auto-creates `DATABASE_URL` variable

#### Step 3: Add API Service (control-plane)
1. Click **+ New** → **GitHub Repo**
2. Select your repo, click **Add Service**
3. Click the service → **Settings** tab:
   - **Service Name**: `api`
   - **Source** section:
     - Dockerfile Path: `docker/Dockerfile.unified`
   - **Build** section:
     - Docker Build Target: `control-plane`
4. **Variables** tab → **Raw Editor** → paste:
   ```
   DATABASE_URL=${{Postgres.DATABASE_URL}}
   SECRETS_ENCRYPTION_KEY=kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=
   AUTH_MODE=none
   DEPLOYMENT_GRADE=preview
   HOST=0.0.0.0
   PORT=9000
   RUST_LOG=info
   ```
5. **Networking** tab → **Generate Domain** (for public access)

#### Step 4: Add Worker Service
1. Click **+ New** → **GitHub Repo** → Select same repo
2. **Settings**:
   - **Service Name**: `worker`
   - Dockerfile Path: `docker/Dockerfile.unified`
   - Docker Build Target: `worker`
3. **Variables**:
   ```
   GRPC_ADDRESS=${{api.RAILWAY_PRIVATE_DOMAIN}}:9001
   RUST_LOG=info
   ```

#### Step 5: Add UI Service
1. Click **+ New** → **GitHub Repo** → Select same repo
2. **Settings**:
   - **Service Name**: `ui`
   - Root Directory: `apps/ui`
   - *(Dockerfile auto-detected from apps/ui/Dockerfile)*
3. **Variables**:
   ```
   PORT=9100
   HOSTNAME=0.0.0.0
   ```
4. **Networking** → **Generate Domain**

#### Step 6: Enable PR Environments
1. Click **Project Settings** (gear icon top-right)
2. Go to **Environments** section
3. Toggle **Enable PR Environments** ON

### Variable Reference

Maps to `docker-compose-full.yaml` environment variables:

| Service | Variable | Value |
|---------|----------|-------|
| api | DATABASE_URL | `${{Postgres.DATABASE_URL}}` |
| api | SECRETS_ENCRYPTION_KEY | `kek-v1:<your-key>` |
| api | AUTH_MODE | `none` |
| api | HOST | `0.0.0.0` |
| api | PORT | `9000` |
| worker | GRPC_ADDRESS | `${{api.RAILWAY_PRIVATE_DOMAIN}}:9001` |
| ui | PORT | `9100` |
| ui | HOSTNAME | `0.0.0.0` |

### Config File

The `railway.toml` at repo root provides default build configuration:

```toml
[build]
builder = "dockerfile"
dockerfilePath = "docker/Dockerfile.unified"

[deploy]
healthcheckPath = "/health"
healthcheckTimeout = 300
restartPolicyType = "on_failure"
restartPolicyMaxRetries = 3
```

Service-specific settings (like Docker target) are configured in Railway dashboard.

## Cost Estimation

### Scale-to-Zero (Default)

Services sleep after 5 minutes of inactivity and wake on incoming requests (~1-2s cold start).

| Resource | Cost |
|----------|------|
| Sleeping services | $0 (no compute) |
| PostgreSQL storage | ~$0.25/GB/month |
| Wake-up compute | Pay only when accessed |

**Dozens of idle PRs**: ~$1-5/month total (storage only)

### Active Usage (when accessed)

| Service | Cost/hour active |
|---------|-----------------|
| Control-plane (0.5 vCPU, 512MB) | ~$0.04 |
| Worker (0.5 vCPU, 512MB) | ~$0.04 |
| UI (0.25 vCPU, 256MB) | ~$0.02 |
| **Total** | **~$0.10/hour** |

With scale-to-zero and $5 monthly credit, cost is minimal even with many concurrent PRs.

## Cleanup Policy

1. **On PR Merge**: Environment deleted automatically by Railway
2. **On PR Close**: Environment deleted automatically by Railway
3. **Manual**: Can delete via Railway dashboard

## Security Considerations

1. **No Production Data**: Preview environments use empty databases
2. **Simplified Auth**: Use `AUTH_MODE=none` for easier testing
3. **Encryption Keys**: Use dedicated preview keys, not production
4. **API Keys**: Use test/sandbox API keys for LLM providers
5. **Network Isolation**: Previews cannot access production resources

## Monitoring

1. **Build Status**: Visible in Railway dashboard and GitHub checks
2. **Health Check**: `/health` endpoint monitored
3. **Logs**: Available in Railway dashboard

## References

- [Railway Environments](https://docs.railway.com/guides/environments)
- [Railway Config as Code](https://docs.railway.com/reference/config-as-code)
- [Railway Dockerfiles](https://docs.railway.com/guides/dockerfiles)
