# PR Preview Environments

## Abstract

This specification defines the PR preview environment system that automatically deploys isolated instances of the Everruns application for each pull request, enabling reviewers to test changes in a production-like environment before merging.

## Requirements

### Functional Requirements

1. **Automatic Deployment**: When a PR is opened or updated, deploy a complete preview environment
2. **Isolation**: Each PR gets its own isolated environment (separate database, services)
3. **Unique URLs**: Each preview has a unique, accessible URL (e.g., `pr-123.preview.everruns.com`)
4. **Automatic Cleanup**: Environments are destroyed when PRs are merged or closed
5. **Comment Integration**: Post deployment URL as a comment on the PR

### Non-Functional Requirements

1. **Cost Efficiency**: Only pay for resources while PR is open
2. **Fast Deployment**: Preview should be available within 5 minutes of push
3. **Minimal Maintenance**: No manual intervention required for standard operation

## Service Options

### Option 1: Railway (Recommended)

**Pros:**
- Native PR environment support via GitHub integration
- Built-in PostgreSQL provisioning
- Automatic cleanup on PR close/merge
- Docker image support (use existing ghcr.io images)
- GitHub Actions integration via [Railway Preview Deploy Action](https://github.com/marketplace/actions/railway-preview-deploy-action)
- $5/month credit on free tier

**Cons:**
- Another vendor to manage
- Limited to their regions

**Implementation:**
- Use Railway's environment feature to create isolated preview per PR
- Each environment gets: control-plane, worker, UI, PostgreSQL
- Environment variables configured per-environment

### Option 2: Render

**Pros:**
- Native preview environments that clone entire stack
- Automatic PostgreSQL cloning with schema migrations
- Blueprint-based infrastructure-as-code
- Automatic cleanup on PR merge/close
- Expiry time for inactive previews

**Cons:**
- Requires Professional workspace ($19/user/month) for full preview features
- More expensive for teams

### Option 3: Fly.io

**Pros:**
- Global edge network (35+ regions)
- Lightweight VM orchestration
- Good for latency-sensitive apps

**Cons:**
- No native PR preview feature
- Requires custom GitHub Actions setup
- More complex configuration

### Comparison Matrix

| Feature | Railway | Render | Fly.io |
|---------|---------|--------|--------|
| Native PR Previews | Yes | Yes (Pro+) | No |
| Auto Cleanup | Yes | Yes | Manual |
| PostgreSQL | Built-in | Built-in | Manual setup |
| Docker Images | Yes | Yes | Yes |
| GitHub Integration | Native + Actions | Native | Actions only |
| Cost | Pay-per-use + $5 credit | Pro plan required | Pay-per-use |
| Ease of Setup | Easy | Easy | Medium |

## Recommended Architecture: Railway

### Service Configuration

```
PR Preview Environment
├── control-plane (Docker: ghcr.io/*/everruns-control-plane:sha-xxx)
│   ├── Port 9000 (REST API)
│   └── Port 9001 (gRPC)
├── worker (Docker: ghcr.io/*/everruns-worker:sha-xxx)
│   └── Connects to control-plane:9001
├── ui (Docker: ghcr.io/*/everruns-ui:sha-xxx)
│   └── Port 9100 (proxies to API)
└── postgres (Railway PostgreSQL)
    └── Auto-provisioned per environment
```

### Environment Variables

```bash
# Database (auto-injected by Railway)
DATABASE_URL=${{Postgres.DATABASE_URL}}

# Security
SECRETS_ENCRYPTION_KEY=kek-v1:preview-environment-key
AUTH_MODE=none  # Simplified for previews

# Worker
GRPC_ADDRESS=control-plane.internal:9001

# Feature flags
DEPLOYMENT_GRADE=preview
```

### GitHub Actions Workflow

```yaml
name: PR Preview

on:
  pull_request:
    types: [opened, synchronize, reopened, closed]

jobs:
  preview:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Deploy Preview
        if: github.event.action != 'closed'
        uses: runletapp/railway-preview-deploy-action@v1
        with:
          railway_token: ${{ secrets.RAILWAY_TOKEN }}
          service: everruns-preview
          cleanup: 'true'

      - name: Comment PR
        if: github.event.action != 'closed'
        uses: actions/github-script@v7
        with:
          script: |
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: '🚀 Preview deployed: https://pr-${{ github.event.number }}.preview.everruns.com'
            })
```

## Implementation Steps

### Phase 1: Railway Setup

1. Create Railway account and project
2. Configure project with services:
   - `control-plane` - Docker from ghcr.io
   - `worker` - Docker from ghcr.io
   - `ui` - Docker from ghcr.io
   - `postgres` - Railway PostgreSQL template
3. Configure environment variables
4. Test manual deployment

### Phase 2: GitHub Integration

1. Generate Railway API token
2. Add `RAILWAY_TOKEN` to GitHub repository secrets
3. Create `.github/workflows/pr-preview.yml`
4. Configure webhook for PR events

### Phase 3: DNS and Routing

1. Configure wildcard DNS for `*.preview.everruns.com`
2. Set up Railway domains per environment
3. Configure SSL certificates (auto via Railway)

### Phase 4: PR Comment Bot

1. Use GitHub Actions to post preview URL
2. Include health check status
3. Update comment on subsequent pushes

## Security Considerations

1. **No Production Data**: Preview environments use empty databases
2. **Simplified Auth**: Use `AUTH_MODE=none` for easier testing
3. **Encryption Keys**: Use dedicated preview keys, not production
4. **API Keys**: Use test/sandbox API keys for LLM providers
5. **Network Isolation**: Previews cannot access production resources

## Cost Estimation

### Railway (per PR, typical usage)

| Service | Estimated Cost |
|---------|---------------|
| Control-plane | ~$2-5/PR/day |
| Worker | ~$1-3/PR/day |
| UI | ~$1-2/PR/day |
| PostgreSQL | ~$1-2/PR/day |
| **Total** | **~$5-12/PR/day** |

With $5 monthly credit and typical PR lifetimes of 1-3 days, cost is minimal for small teams.

## Cleanup Policy

1. **On PR Merge**: Environment deleted immediately
2. **On PR Close**: Environment deleted immediately
3. **Stale PRs**: Environment deleted after 7 days of inactivity (configurable)
4. **Manual**: Can delete via Railway dashboard or CLI

## Monitoring

1. **Deployment Status**: Visible in GitHub PR checks
2. **Health Check**: `/health` endpoint monitored
3. **Logs**: Available in Railway dashboard
4. **Alerts**: Optional Slack/Discord notifications

## References

- [Railway Preview Deploy Action](https://github.com/marketplace/actions/railway-preview-deploy-action)
- [Railway GitHub Actions Tutorial](https://docs.railway.com/tutorials/github-actions-pr-environment)
- [Railway Environments](https://docs.railway.com/guides/environments)
- [Render Preview Environments](https://render.com/docs/preview-environments)
