# ECS Admin Tasks Runbook

This runbook describes how to run administrative tasks in production ECS environments using the admin container.

## Overview

The `everruns-admin` container provides a unified interface for running operational tasks:

| Command | Description | Use Case |
|---------|-------------|----------|
| `migrate` | Run database migrations | Before deployments |
| `migrate-info` | Show migration status | Pre-deployment checks |
| `reencrypt` | Re-encrypt secrets with new key | Key rotation |
| `shell` | Interactive debugging shell | Troubleshooting |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ECS Fargate                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │               everruns-admin container                │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌──────────────┐   │  │
│  │  │   sqlx-cli  │  │  reencrypt  │  │    shell     │   │  │
│  │  │ (migrations)│  │  (secrets)  │  │  (debug)     │   │  │
│  │  └─────────────┘  └─────────────┘  └──────────────┘   │  │
│  └───────────────────────────────────────────────────────┘  │
│                            │                                 │
│                            ▼                                 │
│  ┌───────────────────────────────────────────────────────┐  │
│  │              AWS Secrets Manager                      │  │
│  │  • DATABASE_URL                                       │  │
│  │  • SECRETS_ENCRYPTION_KEY                             │  │
│  │  • SECRETS_ENCRYPTION_KEY_PREVIOUS                    │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
                    ┌───────────────┐
                    │   PostgreSQL  │
                    │   (RDS/etc)   │
                    └───────────────┘
```

## Prerequisites

### AWS Resources

1. **ECS Cluster**: An ECS cluster to run tasks
2. **Task Definition**: The `everruns-admin` task definition registered
3. **Secrets Manager**: Database URL and encryption keys stored
4. **CloudWatch Log Group**: `/ecs/everruns-admin` for task logs
5. **VPC Configuration**: Subnets with database access

### IAM Roles

**Task Execution Role** needs:
```json
{
  "Effect": "Allow",
  "Action": [
    "secretsmanager:GetSecretValue"
  ],
  "Resource": [
    "arn:aws:secretsmanager:*:*:secret:everruns/*"
  ]
}
```

**Task Role** needs:
- Network access to database (via security groups)

### Local Setup

```bash
# Configure environment
export ECS_CLUSTER="everruns"
export ECS_SUBNETS="subnet-xxx,subnet-yyy"
export ECS_SECURITY_GROUP="sg-zzz"
export AWS_REGION="us-east-1"
```

## Running Tasks

### Using the Helper Script

The helper script (`infrastructure/ecs/run-admin-task.sh`) provides a convenient interface:

```bash
# Run migrations
./infrastructure/ecs/run-admin-task.sh migrate

# Check migration status
./infrastructure/ecs/run-admin-task.sh migrate-info

# Dry-run re-encryption
./infrastructure/ecs/run-admin-task.sh reencrypt --dry-run

# Execute re-encryption
./infrastructure/ecs/run-admin-task.sh reencrypt --batch-size 50
```

### Using AWS CLI Directly

For more control, use the AWS CLI:

```bash
# Run migration task
aws ecs run-task \
    --cluster everruns \
    --task-definition everruns-admin \
    --launch-type FARGATE \
    --network-configuration "awsvpcConfiguration={subnets=[subnet-xxx],securityGroups=[sg-zzz],assignPublicIp=DISABLED}" \
    --overrides '{"containerOverrides":[{"name":"admin","command":["migrate"]}]}'
```

### Monitoring Task Progress

```bash
# Watch task status
aws ecs describe-tasks \
    --cluster everruns \
    --tasks <task-arn>

# Stream logs (requires awslogs installed)
aws logs tail /ecs/everruns-admin --follow
```

## Task Reference

### migrate

Runs pending database migrations.

```bash
./infrastructure/ecs/run-admin-task.sh migrate
```

**When to use:**
- Before deploying new application code
- After adding new migration files

**See also:** [Production Migrations Runbook](./production-migrations.md)

### migrate-info

Shows current migration status without making changes.

```bash
./infrastructure/ecs/run-admin-task.sh migrate-info
```

**When to use:**
- Pre-deployment verification
- Debugging migration issues

### reencrypt

Re-encrypts secrets with a new encryption key.

```bash
# Preview changes
./infrastructure/ecs/run-admin-task.sh reencrypt --dry-run

# Execute with batch size
./infrastructure/ecs/run-admin-task.sh reencrypt --batch-size 50

# Process specific table only
./infrastructure/ecs/run-admin-task.sh reencrypt --table llm_providers
```

**When to use:**
- Key rotation
- Emergency key replacement

**See also:** [Encryption Key Rotation Runbook](./encryption-key-rotation.md)

## Building the Admin Container

The admin container is built from the unified Dockerfile:

```bash
# Build locally
docker build --target admin -f docker/Dockerfile.unified -t everruns-admin .

# Push to ECR
aws ecr get-login-password | docker login --username AWS --password-stdin $ECR_REGISTRY
docker tag everruns-admin $ECR_REGISTRY/everruns-admin:latest
docker push $ECR_REGISTRY/everruns-admin:latest
```

### CI/CD Integration

Add to your CI/CD pipeline to build and push the admin container:

```yaml
# Example: GitHub Actions
- name: Build and Push Admin Container
  run: |
    docker build --target admin -f docker/Dockerfile.unified -t $ECR_REGISTRY/everruns-admin:${{ github.sha }} .
    docker push $ECR_REGISTRY/everruns-admin:${{ github.sha }}
```

## Updating the Task Definition

When the admin container image is updated:

1. **Build and push new image** (see above)
2. **Update task definition** with new image tag:

```bash
# Update image tag in task definition
export IMAGE_TAG="new-sha-or-tag"

# Re-register task definition
envsubst < infrastructure/ecs/admin-task-definition.json > /tmp/admin-task.json
aws ecs register-task-definition --cli-input-json file:///tmp/admin-task.json
```

## Troubleshooting

### Task Won't Start

**Symptoms**: Task stays in PENDING or fails immediately

**Check:**
1. Security group allows outbound access
2. Subnets have NAT gateway (for private subnets)
3. Secrets exist and are accessible
4. Task definition is valid

```bash
# Check task failure reason
aws ecs describe-tasks --cluster everruns --tasks <arn> \
    --query 'tasks[0].{status:lastStatus,reason:stoppedReason}'
```

### Task Exits with Non-Zero Code

**Check CloudWatch logs:**

```bash
aws logs get-log-events \
    --log-group-name /ecs/everruns-admin \
    --log-stream-name "admin/<task-id>" \
    --query 'events[*].message'
```

**Common issues:**
- `DATABASE_URL` not set or invalid
- Database not reachable (security group)
- Invalid encryption key format

### Database Connection Issues

**Symptoms**: Task fails with connection timeout

**Check:**
1. Security group allows PostgreSQL port (5432)
2. Database is in same VPC or peered
3. DATABASE_URL is correct

```bash
# Test connectivity from a debug task
./infrastructure/ecs/run-admin-task.sh shell
# Then inside container: curl -v postgres-host:5432
```

## Security Considerations

1. **Secrets**: All sensitive values come from Secrets Manager
2. **Network**: Tasks run in private subnets with no public IP
3. **IAM**: Least-privilege roles for task execution
4. **Logging**: All task output goes to CloudWatch
5. **Audit**: ECS task events provide audit trail

## Related Runbooks

- [Production Migrations](./production-migrations.md) - Database migration procedures
- [Encryption Key Rotation](./encryption-key-rotation.md) - Key rotation procedures
