# Everruns development commands
# Install just: cargo install just (or ./scripts/init-cloud-env.sh for pre-built binary)
# Usage: just <recipe>   (or: just --list)

mod ui
mod docs
mod durable
mod example

# Default recipe: show available commands
default:
    @just --list

# === Setup ===

# Install all development dependencies (Rust tools + UI + Docs)
init:
    ./scripts/lib/setup.sh init

# Install the everruns CLI from local source
install-cli:
    cargo install --path crates/cli --force

# Upload example agents from examples/agents/
upload-agents:
    ./scripts/lib/setup.sh upload-agents

# === Infrastructure Services ===

# Start infrastructure services (Postgres via pg_ctl, Valkey as process)
start-infra:
    ./scripts/lib/infra.sh start

# Stop infrastructure services
stop-infra:
    ./scripts/lib/infra.sh stop

# Stop and remove all infrastructure data (pg data, valkey)
reset:
    ./scripts/lib/infra.sh reset

# === Build & Test ===

# Build all crates
build:
    cargo build

# Run all tests (Rust + UI e2e)
# Note: uses default features only. PostgreSQL-gated tests (postgres-tests, failpoints)
# are run via `just test-integration` which sets up Docker and enables those features.
test:
    cargo test
    cd apps/ui && pnpm run e2e 2>/dev/null || echo "(e2e skipped)"

# Run repository shell-layer tests (scripts/test-*.sh)
test-shell:
    ./scripts/run-shell-tests.sh

# Validate the canonical knowledge/ OKF v0.2 bundle. The upstream linter is
# optional locally and pinned in CI; install it with `just install-okf-lint`.
check-okf:
    #!/usr/bin/env bash
    set -euo pipefail
    python3 scripts/check_okf.py knowledge
    if command -v okf-lint >/dev/null; then
        okf-lint knowledge --max-line-length 10000
    else
        echo "okf-lint not installed — skipping upstream check"
    fi

# Install the upstream OKF v0.2 linter version used by CI.
install-okf-lint:
    cargo install okf-lint --version 0.1.1 --locked

# Run pure unit tests (no PostgreSQL required) - fast feedback
test-unit:
    cargo test -p everruns-anthropic --lib --all-features
    cargo test -p everruns-openai --lib --all-features
    cargo test -p everruns-internal-protocol --lib --all-features
    cargo test -p everruns-core --lib --all-features
    cargo test -p everruns-host --test in_process_runtime_test --test runtime_host_test -- --test-threads=1
    cargo test -p everruns-cli --test auth_integration_test --test chat_integration_test --test files_integration_test -- --test-threads=1

# Run integration tests (requires PostgreSQL via start-infra or externally)
test-integration: start-infra
    #!/usr/bin/env bash
    set -e
    if [ -z "${DATABASE_URL:-}" ]; then
        DB_HOST="${DB_HOST:-localhost}"
        DB_PORT="${DB_PORT:-${PORT_PREFIX:+${PORT_PREFIX}32}}"
        DB_PORT="${DB_PORT:-9332}"
        DB_USER="${DB_USER:-everruns}"
        DB_NAME="${DB_NAME:-everruns_test}"
        export DB_HOST DB_PORT DB_USER DB_NAME
        export DATABASE_URL="postgres://everruns:everruns@${DB_HOST}:${DB_PORT}/${DB_NAME}"
        # Wait for postgres
        for i in {1..30}; do
            if pg_isready -h "${DB_HOST}" -p "${DB_PORT}" -U "${DB_USER}" 2>/dev/null; then break; fi
            sleep 1
        done
        createdb -h "${DB_HOST}" -p "${DB_PORT}" -U "${DB_USER}" "${DB_NAME}" 2>/dev/null || true
    fi
    # Run migrations
    sqlx migrate run --source crates/server/migrations
    # Run tests
    cargo test -p everruns-server --lib
    cargo test -p everruns-server --test api_integration_test -- --test-threads=1
    cargo test -p everruns-server --test repository_integration_test --test repository_conformance_test -- --test-threads=1
    cargo test -p everruns-server \
        --test ag_ui_integration_test \
        --test auth_integration_test \
        --test cli_auth_test \
        --test cli_auth_no_org_test \
        --test client_side_tools_test \
        --test evals_integration_test \
        --test llm_model_default_test \
        --test org_creation_test \
        --test org_isolation_test \
        --test org_lifecycle_test \
        --test schedule_integration_test \
        --test session_git_integration_test \
        -- --test-threads=1
    cargo test -p everruns-durable --test postgres_integration_test --features postgres-tests -- --test-threads=1
    cargo test -p everruns-durable --test postgres_repository_test --features postgres-tests -- --test-threads=1
    cargo test -p everruns-durable --test failure_injection_test --features "failpoints,postgres-tests" -- --test-threads=1

# Run workflow tests (requires running server + worker)
test-workflow:
    cargo test -p everruns-server --test workflow_test -- --test-threads=1

# Run LLM tests against real APIs (provider API keys required; MODEL_API_KEY covers Meta)
# Skip providers: SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini just test-llm
test-llm:
    cargo test -p everruns-core --test agent_run_basic
    cargo test -p everruns-core --test agent_run_with_thinking

# Run all formatters and linters (auto-fix)
fmt:
    cargo fmt
    cargo clippy --all-targets --fix --allow-dirty --allow-staged 2>/dev/null || true
    cd apps/ui && pnpm run format 2>/dev/null || true
    cd apps/ui && pnpm run lint -- --fix 2>/dev/null || true

# Run format, lint, and test checks
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Run changed-surface checks with a dedicated Rust cache shared across worktrees.
pre-push:
    ./scripts/lib/pre-push.sh

# Run every pre-push check regardless of the changed files.
pre-push-full:
    EVERRUNS_PRE_PUSH_FULL=1 ./scripts/lib/pre-push.sh

# Run all pre-PR checks (fmt, clippy, tests, UI, OpenAPI, docs)
pre-pr:
    ./scripts/lib/pre-pr.sh

# Clean build artifacts
clean:
    cargo clean

# === Services ===

# Start in DEV MODE (in-memory storage, no PostgreSQL required)
start-dev *args:
    ./scripts/lib/services.sh start-dev {{args}}

# Start everything with auto-reload (Postgres, Valkey, API, Worker, UI)
start-all *args:
    ./scripts/lib/services.sh start-all {{args}}

# Start in PRODUCTION MODE (release builds, no watchers, production UI)
start-production *args:
    ./scripts/lib/services.sh start-production {{args}}

# Stop all services (API, Worker, UI, Postgres, Valkey)
stop-all:
    ./scripts/lib/services.sh stop-all

# SeaweedFS local S3 for object-storage testing: start|stop|reset|bucket|logs (knowledge/runtime-resources/object-storage.md)
seaweedfs cmd="start":
    ./scripts/lib/seaweedfs.sh {{cmd}}

# === Load Testing ===

# Load test subcommand: just load-test <profile> [args]
# Profiles: quick, medium, heavy
# Example: just load-test medium
#          just load-test quick --help
#          SESSIONS=200 just load-test medium
[no-cd]
load-test profile="medium" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{profile}}" in
        quick)
            SESSIONS="${SESSIONS:-10}" \
            MESSAGES_PER_SESSION="${MESSAGES_PER_SESSION:-10}" \
            MAX_CONCURRENT="${MAX_CONCURRENT:-10}" \
            cargo bench --package everruns-server --bench load_test -- {{args}}
            ;;
        medium)
            SESSIONS="${SESSIONS:-100}" \
            MESSAGES_PER_SESSION="${MESSAGES_PER_SESSION:-50}" \
            MAX_CONCURRENT="${MAX_CONCURRENT:-50}" \
            cargo bench --package everruns-server --bench load_test -- {{args}}
            ;;
        heavy)
            SESSIONS="${SESSIONS:-500}" \
            MESSAGES_PER_SESSION="${MESSAGES_PER_SESSION:-100}" \
            MAX_CONCURRENT="${MAX_CONCURRENT:-100}" \
            cargo bench --package everruns-server --bench load_test -- {{args}}
            ;;
        *)
            echo "Unknown profile: {{profile}}"
            echo "Available profiles: quick, medium, heavy"
            echo ""
            echo "Usage: just load-test <profile> [args]"
            echo ""
            echo "Profiles:"
            echo "  quick     - 10 sessions, 10 messages (100 total)"
            echo "  medium    - 100 sessions, 50 messages (5000 total) [default]"
            echo "  heavy     - 500 sessions, 100 messages (50000 total)"
            echo ""
            echo "Examples:"
            echo "  just load-test quick"
            echo "  just load-test heavy"
            echo "  SESSIONS=200 just load-test medium"
            exit 1
            ;;
    esac
