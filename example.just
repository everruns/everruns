# Example docker-compose commands

# Start example docker-compose-full (checks local compose not running)
# Optional: specify image tag (e.g., `just example start v0.1.0`)
start tag="latest":
    #!/usr/bin/env bash
    set -euo pipefail
    # Check if local docker compose containers are running (not from example compose)
    # We check the com.docker.compose.project label to distinguish local vs example containers
    local_postgres=$(docker ps --filter "name=^everruns-postgres$" --filter "label=com.docker.compose.project=local" --format '{{"{{"}}.Names{{"}}"}}' 2>/dev/null || true)
    local_jaeger=$(docker ps --filter "name=^everruns-jaeger$" --filter "label=com.docker.compose.project=local" --format '{{"{{"}}.Names{{"}}"}}' 2>/dev/null || true)
    if [[ -n "$local_postgres" || -n "$local_jaeger" ]]; then
        echo "❌ Local docker compose appears to be running (found containers from 'local' project)."
        echo "   Stop it first with: just stop-docker"
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

    echo "✅ Local docker compose not running, starting example full stack..."
    cd examples
    docker compose -f docker-compose-full.yaml up -d
    echo "✅ Example full stack started!"
    echo "   - UI: http://localhost:8080"
    echo "   - Jaeger UI: http://localhost:16686"

# Stop example docker-compose-full
stop:
    cd examples && docker compose -f docker-compose-full.yaml down

# Reset example docker-compose-full (removes volumes)
reset:
    cd examples && docker compose -f docker-compose-full.yaml down -v

# Follow logs from example docker-compose-full
logs *args:
    cd examples && docker compose -f docker-compose-full.yaml logs -f {{args}}

# Pull latest images for example docker-compose-full
# Optional: specify image tag (e.g., `just example pull v0.1.0`)
pull tag="latest":
    export EVERRUNS_TAG={{tag}} && cd examples && docker compose -f docker-compose-full.yaml pull
