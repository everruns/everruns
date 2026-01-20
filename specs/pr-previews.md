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

### Service Configuration

Configure these services in Railway dashboard:

```
PR Preview Environment
├── control-plane
│   ├── Dockerfile: docker/Dockerfile.unified
│   ├── Target: control-plane
│   └── Port 9000 (REST API)
├── worker
│   ├── Dockerfile: docker/Dockerfile.unified
│   ├── Target: worker
│   └── Connects to control-plane via gRPC
├── ui
│   ├── Root Directory: apps/ui
│   ├── Dockerfile: apps/ui/Dockerfile
│   └── Port 9100
└── postgres (Railway PostgreSQL template)
```

### Setup Steps

1. **Create Railway Project**
   - New Project → Empty Project
   - Add services as described above

2. **Connect GitHub**
   - Project Settings → Integrations → Connect GitHub
   - Select the repository

3. **Enable PR Environments**
   - Project Settings → Environments
   - Enable "PR Environments"
   - Railway auto-creates environment per PR

4. **Configure Services**

   For each service, set in Railway dashboard:

   **control-plane:**
   - Dockerfile Path: `docker/Dockerfile.unified`
   - Docker Build Target: `control-plane`
   - Variables:
     ```
     DATABASE_URL=${{Postgres.DATABASE_URL}}
     SECRETS_ENCRYPTION_KEY=kek-v1:<generate-key>
     AUTH_MODE=none
     DEPLOYMENT_GRADE=preview
     ```

   **worker:**
   - Dockerfile Path: `docker/Dockerfile.unified`
   - Docker Build Target: `worker`
   - Variables:
     ```
     GRPC_ADDRESS=${{control-plane.RAILWAY_PRIVATE_DOMAIN}}:9001
     ```

   **ui:**
   - Root Directory: `apps/ui`
   - Variables:
     ```
     API_URL=http://${{control-plane.RAILWAY_PRIVATE_DOMAIN}}:9000
     ```

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
