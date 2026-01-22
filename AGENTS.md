## Coding-agent guidance

### Style

Telegraph. Drop filler/grammar. Min tokens.

### Critical Thinking

Fix root cause. Unsure: read more code; if stuck, ask w/ short options. Unrecognized changes: assume other agent; keep going. If causes issues, stop + ask.

### Principles

- Important decisions as comments on top of file
- Code testable, smoke testable, runnable locally
- Small, incremental PR-sized changes
- No backward compat needed (internal code)

### Quick Reference

| Resource | Location |
|----------|----------|
| Specs | `specs/` - architecture, models, APIs, etc. |
| Skills | `.claude/skills/` - smoke-test, ui-screenshots |
| Test cases | `test_cases/` - format in `specs/conventions.md` |
| Docs source | `docs/` → published at docs.everruns.com |
| PR template | `.github/pull_request_template.md` |

### Cloud Agent Start

```bash
./scripts/init-cloud-env.sh       # Install just + gh
just start-dev --no-watch         # DEV MODE (no Docker)
```

Pre-configured: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`

### Local Dev

```bash
just start-dev          # DEV MODE (in-memory, no Docker)
just start-all          # Full mode (PostgreSQL)
just --list             # All commands
```

### Rust

- Stable Rust (edition 2024), toolchain in `rust-toolchain.toml`
- `cargo fmt` and `cargo clippy -- -D warnings` for touched crates

### Pre-PR Checklist

1. `just pre-pr` (runs 2-7 automatically)
2. `cargo fmt --check`
3. `cargo clippy --all-targets --all-features -- -D warnings`
4. `cargo test --all-features`
5. `npm run lint` + `npm run build` in `apps/ui/`
6. OpenAPI spec fresh: `./scripts/export-openapi.sh`
7. Docs build: `npm run build` in `apps/docs/`
8. Rebase on main: `git fetch origin main && git rebase origin/main`
9. Smoke test new functionality
10. UI screenshots for UI changes (use `.claude/skills/ui-screenshots/`)
11. CI green before merge
12. Resolve all PR comments

### CI

- GitHub Actions. Check via `gh` tool.
- **NEVER merge when CI is red.** No exceptions.

### Commits

[Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`

Types: feat, fix, docs, refactor, test, chore

### PRs

**REQUIRED:** Use `.github/pull_request_template.md`. Squash and Merge.

See `specs/conventions.md` for details.
