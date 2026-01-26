# Example docker-compose commands

# Start example docker-compose-full (checks local compose not running)
start:
    #!/usr/bin/env bash
    set -euo pipefail
    # Check if local docker compose containers are running
    if docker ps --format '{{"{{"}}.Names{{"}}"}}' 2>/dev/null | grep -q '^everruns-postgres$\|^everruns-jaeger$'; then
        echo "❌ Local docker compose appears to be running (found everruns-postgres or everruns-jaeger)."
        echo "   Stop it first with: just stop-docker"
        exit 1
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
