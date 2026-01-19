---
title: PR Preview Environments Setup
description: How to configure Railway for automatic PR preview deployments
---

# PR Preview Environments Setup Guide

This guide explains how to set up automatic preview deployments for pull requests using Railway.

## Overview

PR previews automatically deploy an isolated instance of Everruns for each pull request, allowing reviewers to test changes before merging. The preview includes:
- Control-plane (REST API + gRPC server)
- Worker (durable task execution)
- UI (Next.js frontend)
- PostgreSQL database (isolated per PR)

## Prerequisites

- Railway account (https://railway.app)
- GitHub repository admin access
- Docker images published to ghcr.io (already configured via CI)

## Setup Steps

### 1. Create Railway Project

1. Log in to Railway dashboard
2. Create a new project: **New Project** → **Empty Project**
3. Name it `everruns-previews` (or similar)

### 2. Add Services

Create the following services in Railway:

#### PostgreSQL Database

1. Click **+ New** → **Database** → **PostgreSQL**
2. Railway auto-configures `DATABASE_URL`

#### Control-Plane Service

1. Click **+ New** → **Docker Image**
2. Configure:
   - Image: `ghcr.io/<your-org>/everruns-control-plane:${{IMAGE_TAG}}`
   - Environment variables:
     ```
     DATABASE_URL=${{Postgres.DATABASE_URL}}
     SECRETS_ENCRYPTION_KEY=kek-v1:<generate-a-key>
     AUTH_MODE=none
     DEPLOYMENT_GRADE=preview
     GRPC_ADDRESS=0.0.0.0:9001
     ```
3. Add internal networking for gRPC (port 9001)

#### Worker Service

1. Click **+ New** → **Docker Image**
2. Configure:
   - Image: `ghcr.io/<your-org>/everruns-worker:${{IMAGE_TAG}}`
   - Environment variables:
     ```
     GRPC_ADDRESS=control-plane.railway.internal:9001
     ```
   - Start command: worker runs automatically

#### UI Service

1. Click **+ New** → **Docker Image**
2. Configure:
   - Image: `ghcr.io/<your-org>/everruns-ui:${{IMAGE_TAG}}`
   - Environment variables:
     ```
     API_URL=http://control-plane.railway.internal:9000
     ```
3. Generate a public domain for this service

### 3. Configure GitHub Integration

#### Generate Railway Token

1. In Railway, go to **Account Settings** → **Tokens**
2. Create a new token with project access
3. Copy the token

#### Add GitHub Secrets

In your GitHub repository:

1. Go to **Settings** → **Secrets and variables** → **Actions**
2. Add repository secret:
   - Name: `RAILWAY_TOKEN`
   - Value: (paste Railway token)
3. Add repository variable:
   - Name: `RAILWAY_PROJECT_ID`
   - Value: (copy from Railway project settings)

### 4. Enable the Workflow

The PR preview workflow is already configured at `.github/workflows/pr-preview.yml`. It will:

1. Wait for Docker images to be built
2. Create a new Railway environment for each PR
3. Deploy services with PR-specific image tags
4. Post the preview URL as a PR comment
5. Delete the environment when PR is closed/merged

## Verification

1. Create a test PR
2. Wait for CI to build Docker images (~5-10 min)
3. Wait for PR preview deployment (~2-3 min)
4. Check PR comments for preview URL
5. Visit the preview URL and verify:
   - UI loads correctly
   - API health check passes (`/health`)
   - Can create agents and sessions

## Troubleshooting

### Preview Not Deploying

1. Check GitHub Actions logs for errors
2. Verify `RAILWAY_TOKEN` secret is set correctly
3. Ensure Docker images were published successfully

### Preview URL Not Working

1. Check Railway dashboard for service status
2. Verify environment variables are set
3. Check service logs in Railway

### Database Connection Errors

1. Verify PostgreSQL service is running in Railway
2. Check `DATABASE_URL` is correctly referenced
3. Run migrations if needed (admin container)

## Cost Management

Railway charges based on usage. To minimize costs:

- Previews are automatically deleted on PR close
- Consider setting up inactive preview expiry
- Monitor usage in Railway dashboard

## Security Notes

- Preview environments use `AUTH_MODE=none` for easier testing
- Use a dedicated encryption key (not production)
- Do not seed with production data
- Preview URLs are public (share carefully)

## Related

- [PR Preview Specification](/specs/pr-previews.md)
- [Railway Documentation](https://docs.railway.com)
- [Railway Environments](https://docs.railway.com/guides/environments)
