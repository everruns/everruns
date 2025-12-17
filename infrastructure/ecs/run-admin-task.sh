#!/bin/bash
# Run an admin task on ECS Fargate
#
# Usage:
#   ./run-admin-task.sh migrate
#   ./run-admin-task.sh reencrypt --dry-run
#   ./run-admin-task.sh migrate-info
#
# Environment variables:
#   ECS_CLUSTER       - ECS cluster name (default: everruns)
#   ECS_SUBNETS       - Comma-separated subnet IDs
#   ECS_SECURITY_GROUP - Security group ID for the task
#   AWS_REGION        - AWS region (default: us-east-1)

set -e

# Configuration
CLUSTER="${ECS_CLUSTER:-everruns}"
REGION="${AWS_REGION:-us-east-1}"
TASK_DEFINITION="everruns-admin"

# Validate required environment variables
if [ -z "$ECS_SUBNETS" ]; then
    echo "ERROR: ECS_SUBNETS environment variable is required"
    echo "Example: export ECS_SUBNETS='subnet-123,subnet-456'"
    exit 1
fi

if [ -z "$ECS_SECURITY_GROUP" ]; then
    echo "ERROR: ECS_SECURITY_GROUP environment variable is required"
    echo "Example: export ECS_SECURITY_GROUP='sg-123456'"
    exit 1
fi

# Get command and arguments
if [ $# -lt 1 ]; then
    echo "Usage: $0 <command> [args...]"
    echo ""
    echo "Commands:"
    echo "  migrate           Run database migrations"
    echo "  migrate-info      Show migration status"
    echo "  reencrypt         Re-encrypt secrets (add --dry-run for preview)"
    echo ""
    echo "Examples:"
    echo "  $0 migrate"
    echo "  $0 reencrypt --dry-run"
    echo "  $0 reencrypt --batch-size 50"
    exit 1
fi

COMMAND="$1"
shift
ARGS="$@"

# Build the command override
# Convert command and args to JSON array format
OVERRIDE_CMD=$(printf '%s\n' "$COMMAND" $ARGS | jq -R . | jq -s .)

echo "Running admin task on ECS..."
echo "  Cluster: $CLUSTER"
echo "  Command: $COMMAND $ARGS"
echo ""

# Run the ECS task
TASK_ARN=$(aws ecs run-task \
    --cluster "$CLUSTER" \
    --task-definition "$TASK_DEFINITION" \
    --launch-type FARGATE \
    --network-configuration "awsvpcConfiguration={subnets=[${ECS_SUBNETS}],securityGroups=[${ECS_SECURITY_GROUP}],assignPublicIp=DISABLED}" \
    --overrides "{\"containerOverrides\":[{\"name\":\"admin\",\"command\":$OVERRIDE_CMD}]}" \
    --region "$REGION" \
    --query 'tasks[0].taskArn' \
    --output text)

if [ -z "$TASK_ARN" ] || [ "$TASK_ARN" = "None" ]; then
    echo "ERROR: Failed to start task"
    exit 1
fi

TASK_ID=$(echo "$TASK_ARN" | rev | cut -d'/' -f1 | rev)

echo "Task started: $TASK_ID"
echo "Task ARN: $TASK_ARN"
echo ""
echo "Waiting for task to complete..."

# Wait for task to complete
aws ecs wait tasks-stopped \
    --cluster "$CLUSTER" \
    --tasks "$TASK_ARN" \
    --region "$REGION"

# Get task result
TASK_RESULT=$(aws ecs describe-tasks \
    --cluster "$CLUSTER" \
    --tasks "$TASK_ARN" \
    --region "$REGION" \
    --query 'tasks[0]')

EXIT_CODE=$(echo "$TASK_RESULT" | jq -r '.containers[0].exitCode // "unknown"')
STOP_REASON=$(echo "$TASK_RESULT" | jq -r '.stoppedReason // "N/A"')

echo ""
echo "Task completed!"
echo "  Exit code: $EXIT_CODE"
echo "  Stop reason: $STOP_REASON"
echo ""

# Show logs
LOG_GROUP="/ecs/everruns-admin"
LOG_STREAM="admin/${TASK_ID}"

echo "Fetching logs from CloudWatch..."
echo "  Log group: $LOG_GROUP"
echo "  Log stream: $LOG_STREAM"
echo ""
echo "--- Task Output ---"

aws logs get-log-events \
    --log-group-name "$LOG_GROUP" \
    --log-stream-name "$LOG_STREAM" \
    --region "$REGION" \
    --query 'events[*].message' \
    --output text 2>/dev/null || echo "(No logs available yet - check CloudWatch manually)"

echo "--- End Output ---"

# Exit with the task's exit code
if [ "$EXIT_CODE" = "0" ]; then
    echo ""
    echo "Task completed successfully!"
    exit 0
else
    echo ""
    echo "Task failed with exit code: $EXIT_CODE"
    exit 1
fi
