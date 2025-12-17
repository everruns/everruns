#!/bin/bash
# Admin container entrypoint for everruns
# Supports migrations, key re-encryption, and other admin tasks

set -e

COMMAND=${1:-help}
shift || true

case "$COMMAND" in
    migrate)
        echo "Running database migrations..."
        if [ -z "$DATABASE_URL" ]; then
            echo "ERROR: DATABASE_URL environment variable is required"
            exit 1
        fi
        sqlx migrate run --source /app/migrations "$@"
        echo "Migrations completed successfully"
        ;;

    migrate-info)
        echo "Showing migration status..."
        if [ -z "$DATABASE_URL" ]; then
            echo "ERROR: DATABASE_URL environment variable is required"
            exit 1
        fi
        sqlx migrate info --source /app/migrations "$@"
        ;;

    reencrypt)
        echo "Running secrets re-encryption..."
        if [ -z "$DATABASE_URL" ]; then
            echo "ERROR: DATABASE_URL environment variable is required"
            exit 1
        fi
        if [ -z "$SECRETS_ENCRYPTION_KEY" ]; then
            echo "ERROR: SECRETS_ENCRYPTION_KEY environment variable is required"
            exit 1
        fi
        /app/reencrypt-secrets "$@"
        echo "Re-encryption completed successfully"
        ;;

    shell)
        echo "Starting interactive shell..."
        exec /bin/bash
        ;;

    help|--help|-h)
        cat << 'EOF'
Everruns Admin Container

USAGE:
    docker run everruns-admin <COMMAND> [OPTIONS]

COMMANDS:
    migrate         Run database migrations
    migrate-info    Show migration status
    reencrypt       Re-encrypt secrets with new key
    shell           Start interactive shell (for debugging)
    help            Show this help message

ENVIRONMENT VARIABLES:
    DATABASE_URL                    PostgreSQL connection string (required for most commands)
    SECRETS_ENCRYPTION_KEY          Primary encryption key (required for reencrypt)
    SECRETS_ENCRYPTION_KEY_PREVIOUS Previous encryption key (optional, for rotation)
    RUST_LOG                        Log level (default: info)

EXAMPLES:
    # Run migrations
    docker run --rm \
        -e DATABASE_URL="postgres://user:pass@host:5432/db" \
        everruns-admin migrate

    # Check migration status
    docker run --rm \
        -e DATABASE_URL="postgres://user:pass@host:5432/db" \
        everruns-admin migrate-info

    # Dry-run key re-encryption
    docker run --rm \
        -e DATABASE_URL="postgres://user:pass@host:5432/db" \
        -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
        -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
        everruns-admin reencrypt --dry-run

    # Execute key re-encryption
    docker run --rm \
        -e DATABASE_URL="postgres://user:pass@host:5432/db" \
        -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
        -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
        everruns-admin reencrypt --batch-size 50
EOF
        ;;

    *)
        echo "Unknown command: $COMMAND"
        echo "Run 'help' for usage information"
        exit 1
        ;;
esac
