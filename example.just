# Example docker-compose commands

# Start example Docker Compose stack (checks local compose not running)
# Optional: specify image tag (e.g., `just example start v0.1.0`)
start tag="latest":
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/lib/common.sh
    apply_port_prefix_defaults
    : "${EXAMPLE_PROXY_PORT:=$PROXY_PORT}"
    export VICTORIAMETRICS_PORT
    : "${EXAMPLE_COMPOSE_PROJECT_NAME:=${COMPOSE_PROJECT_NAME}-example}"
    # Check if local docker compose containers are running (not from example compose)
    # We check the com.docker.compose.project label to distinguish local vs example containers
    local_postgres=$(docker ps --filter "label=com.docker.compose.project=$COMPOSE_PROJECT_NAME" --filter "label=com.docker.compose.service=postgres" --format '{{"{{"}}.Names{{"}}"}}' 2>/dev/null || true)
    if [[ -n "$local_postgres" ]]; then
        echo "❌ Local docker compose appears to be running (project '$COMPOSE_PROJECT_NAME')."
        echo "   Stop it first with: just stop-infra"
        exit 1
    fi

    # Set image tag
    export EVERRUNS_TAG="{{tag}}"
    echo "✅ Using image tag: $EVERRUNS_TAG"

    # Pass through API keys from environment if set
    if [[ -n "${OPENAI_API_KEY:-}" ]]; then
        echo "✅ OPENAI_API_KEY found, setting DEFAULT_OPENAI_API_KEY"
        export DEFAULT_OPENAI_API_KEY="$OPENAI_API_KEY"
    fi
    if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
        echo "✅ ANTHROPIC_API_KEY found, setting DEFAULT_ANTHROPIC_API_KEY"
        export DEFAULT_ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"
    fi
    if [[ -n "${GEMINI_API_KEY:-}" ]]; then
        echo "✅ GEMINI_API_KEY found, setting DEFAULT_GEMINI_API_KEY"
        export DEFAULT_GEMINI_API_KEY="$GEMINI_API_KEY"
    fi

    # Pass through worker gRPC auth token if set
    if [[ -n "${WORKER_GRPC_AUTH_TOKEN:-}" ]]; then
        echo "✅ WORKER_GRPC_AUTH_TOKEN found"
        export WORKER_GRPC_AUTH_TOKEN="$WORKER_GRPC_AUTH_TOKEN"
    fi

    echo "✅ Local docker compose not running, starting example full stack..."
    cd infra/docker-compose
    docker compose --project-name "$EXAMPLE_COMPOSE_PROJECT_NAME" -f docker-compose.yaml up --pull always -d
    echo "✅ Example full stack started!"
    echo "   - UI: http://localhost:${EXAMPLE_PROXY_PORT}"
    echo "   - VictoriaMetrics: http://localhost:${VICTORIAMETRICS_PORT:-8428}/vmui"

# Stop example Docker Compose stack
stop:
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/lib/common.sh
    apply_port_prefix_defaults
    export EXAMPLE_COMPOSE_PROJECT_NAME="${EXAMPLE_COMPOSE_PROJECT_NAME:-${COMPOSE_PROJECT_NAME}-example}"
    cd infra/docker-compose
    docker compose --project-name "$EXAMPLE_COMPOSE_PROJECT_NAME" -f docker-compose.yaml down

# Reset example Docker Compose stack (removes volumes)
reset:
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/lib/common.sh
    apply_port_prefix_defaults
    export EXAMPLE_COMPOSE_PROJECT_NAME="${EXAMPLE_COMPOSE_PROJECT_NAME:-${COMPOSE_PROJECT_NAME}-example}"
    cd infra/docker-compose
    docker compose --project-name "$EXAMPLE_COMPOSE_PROJECT_NAME" -f docker-compose.yaml down -v

# Follow logs from example Docker Compose stack
logs *args:
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/lib/common.sh
    apply_port_prefix_defaults
    export EXAMPLE_COMPOSE_PROJECT_NAME="${EXAMPLE_COMPOSE_PROJECT_NAME:-${COMPOSE_PROJECT_NAME}-example}"
    cd infra/docker-compose
    docker compose --project-name "$EXAMPLE_COMPOSE_PROJECT_NAME" -f docker-compose.yaml logs -f {{args}}

# Pull latest images for example Docker Compose stack
# Optional: specify image tag (e.g., `just example pull v0.1.0`)
pull tag="latest":
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/lib/common.sh
    apply_port_prefix_defaults
    export EVERRUNS_TAG={{tag}}
    export EXAMPLE_COMPOSE_PROJECT_NAME="${EXAMPLE_COMPOSE_PROJECT_NAME:-${COMPOSE_PROJECT_NAME}-example}"
    cd infra/docker-compose
    docker compose --project-name "$EXAMPLE_COMPOSE_PROJECT_NAME" -f docker-compose.yaml pull
