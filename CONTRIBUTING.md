# Contributing to Everruns

## Getting Started

See [README.md](./README.md) for quick start instructions.

## Code Quality

Before committing, run:

```bash
./scripts/dev.sh check
```

This runs:
- `cargo fmt --check` - Code formatting
- `cargo clippy --all-targets -- -D warnings` - Linting
- `cargo test` - Tests

## Architecture Principles

1. **No Temporal concepts in public API**: Never expose workflow IDs, task queues, etc.
2. **Event sourcing**: All run events are persisted for replay via SSE
3. **Restart-safe**: All state in Postgres, workflows are durable

## Coding Guidelines

### Rust

- Follow standard Rust naming conventions
- Use `rustfmt` for formatting (enforced by CI)
- Address all clippy warnings
- Use `anyhow::Result` for application errors
- Use `thiserror` for domain-specific error types
- Never use `.unwrap()` in production code

### TypeScript (UI)

- Use TypeScript strict mode
- Follow Next.js App Router conventions
- Use React Query for data fetching
- Use shadcn/ui components

## Database Migrations

```bash
# Create a new migration
sqlx migrate add -r <migration_name>

# Run migrations
./scripts/dev.sh migrate
```

Rules:
- Migrations live in `crates/everruns-storage/migrations/`
- Never modify existing migrations - always add new ones
- Test migrations both up and down

## Testing

```bash
# Unit tests
cargo test

# Smoke tests (requires services running)
./scripts/dev.sh smoke-test

# With UI
./scripts/dev.sh smoke-test --with-ui
```

See [SMOKE_TEST.md](./SMOKE_TEST.md) for detailed testing guide.

## License Compliance

We use `cargo-deny` to ensure permissive licenses only.

**Allowed**: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC

**Prohibited**: GPL, AGPL

## Debugging

```bash
# Temporal UI
open http://localhost:8080

# Database access
docker exec -it everruns-postgres psql -U everruns -d everruns

# Verbose logs
RUST_LOG=debug ./scripts/dev.sh api
```

## Getting Help

- See [PLAN.md](./PLAN.md) for development roadmap
- Open an issue for bugs or feature requests
