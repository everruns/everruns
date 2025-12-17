# ECS Infrastructure

This directory contains ECS task definitions and helper scripts for running everruns services.

## Admin Task

The admin task is used for running one-off operations like database migrations and key rotation.

### Files

- `admin-task-definition.json` - ECS task definition template for admin tasks
- `run-admin-task.sh` - Helper script for running admin tasks

### Setup

1. **Register the task definition** (replace placeholders first):

```bash
# Set your values
export ECR_REGISTRY="123456789.dkr.ecr.us-east-1.amazonaws.com"
export IMAGE_TAG="latest"
export EXECUTION_ROLE_ARN="arn:aws:iam::123456789:role/ecsTaskExecutionRole"
export TASK_ROLE_ARN="arn:aws:iam::123456789:role/everruns-task-role"
export DATABASE_URL_SECRET_ARN="arn:aws:secretsmanager:us-east-1:123456789:secret:everruns/database-url"
export ENCRYPTION_KEY_SECRET_ARN="arn:aws:secretsmanager:us-east-1:123456789:secret:everruns/encryption-key"
export ENCRYPTION_KEY_PREVIOUS_SECRET_ARN="arn:aws:secretsmanager:us-east-1:123456789:secret:everruns/encryption-key-previous"
export AWS_REGION="us-east-1"

# Generate task definition from template
envsubst < admin-task-definition.json > /tmp/admin-task.json

# Register
aws ecs register-task-definition --cli-input-json file:///tmp/admin-task.json
```

2. **Create CloudWatch log group**:

```bash
aws logs create-log-group --log-group-name /ecs/everruns-admin
```

3. **Configure the helper script**:

```bash
export ECS_CLUSTER="everruns"
export ECS_SUBNETS="subnet-123,subnet-456"
export ECS_SECURITY_GROUP="sg-123456"
export AWS_REGION="us-east-1"
```

### Usage

See the [Production Migrations Runbook](../../doc/sre/runbooks/production-migrations.md) for detailed procedures.

```bash
# Run migrations
./run-admin-task.sh migrate

# Check migration status
./run-admin-task.sh migrate-info

# Dry-run key re-encryption
./run-admin-task.sh reencrypt --dry-run

# Execute key re-encryption
./run-admin-task.sh reencrypt --batch-size 50
```

### Required IAM Permissions

The task execution role needs:
- `secretsmanager:GetSecretValue` for the secrets
- `logs:CreateLogStream`, `logs:PutLogEvents` for CloudWatch

The task role needs:
- Database network access (via security groups)
- Any additional permissions for your operations
