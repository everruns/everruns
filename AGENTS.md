## Everruns

Agentic runtime and control plane: Rust workspace in `crates/` (edition 2024), Next.js UI in
`apps/ui/`, docs site in `apps/docs/` (content in `docs/`), embeddable examples in `examples/`.
`just --list` shows every dev command.

### Where guidance lives

Keep each fact in exactly one layer, and read the layer that owns it.

| Layer | Owns |
|---|---|
| `AGENTS.md` (this file, plus per-subtree ones) | repo gotchas that are not discoverable from the code |
| `knowledge/` | why and what: OKF v0.2 design intent, contracts, success bars — start at `knowledge/index.md` |
| `.agents/skills/` | how: workflows loaded on demand (`/ship`, `/maintenance`, `/process-issues`, `/manual-ui-testing`) |
| source, `justfile`, `.github/workflows/` | exact commands, fields, shapes, and checks |

Extend the owning layer instead of restating it here. When working in a subtree, check for a
closer `AGENTS.md` (`apps/ui/`, `crates/server/migrations/`, `plugins/`, `.deepsec/`).

### Norms

- Telegraph. Drop filler. Keep updates short and factual.
- Start from latest `origin/main` unless the task says otherwise.
- Keep changes small, PR-sized, testable, and runnable locally.
- For bug fixes, write or update a failing test before the fix when practical.
- Record important decisions as concise comments near the relevant code, not in scratch docs.
- Internal code needs no backward compatibility unless a spec says otherwise.

### Gotchas

- Working-tree changes you did not make are probably another agent's or the user's. Work with
  them, and stage files by name rather than `git add -A`.
- Rebases silently keep colliding migration numbers. After a rebase that touches
  `crates/server/migrations/`, run `bash scripts/lib/check-migration-ordering.sh` and renumber.
- Run `just pre-push` before pushing. It scopes expensive checks to changed surfaces and shares a
  dedicated pre-push Rust target across worktrees; use `just pre-push-full` to force every check.
- Knowledge captures why/what; link to source instead of copying fields, enum variants, SQL DDL,
  or API shapes. `docs/` holds public product documentation only — durable decisions and
  investigations belong in `knowledge/` or `proposals/`. Run `just check-okf` after knowledge
  changes.
- Linear: OSS project, EVE team.

### Local dev

```bash
# Canonical coding-agent stack; choose a distinct prefix per worktree.
PORT_PREFIX=271 AUTH_MODE=none ./scripts/start-agent-dev.sh
```

This is the single agent startup contract. The script preserves the caller's `PORT_PREFIX` and
`AUTH_MODE` through Doppler, then runs `just start-all --no-watch`. It starts PostgreSQL, Valkey,
and NATS as local processes (no Docker), plus the API, worker, UI, and Caddy. The proxy is
`http://localhost/<PORT_PREFIX>00` (prefix `271` → `http://localhost:27100`). Pick an unused prefix
from `1` through `654`; never reuse one across concurrent worktrees.

Prerequisites: Rust and Node.js, a configured Doppler CLI, and the repository dependencies. On a
first checkout run `./scripts/init-cloud-env.sh` and then `just init`. The first Rust and Next.js
build can take several minutes.

`AUTH_MODE=none` is local-development-only and gives the anonymous user admin access. For
authenticated testing, explicitly select `admin`, `full`, or `external` and configure that mode's
variables per `docs/sre/runbooks/authentication.md`.

`start-all` maps Doppler's `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` to
`DEFAULT_OPENAI_API_KEY` and `DEFAULT_ANTHROPIC_API_KEY`. Analyze/Health additionally requires
`UTILITY_OPENAI_API_KEY`; no extra setup is needed when Doppler already supplies it. If Doppler does
not supply `SECRETS_ENCRYPTION_KEY`, startup uses the stable repository local-development key.

PostgreSQL and NATS data persist under `.local/data/`, isolated by the derived ports. Encrypted
database records depend on the encryption key remaining stable: the default is the public,
non-production key from `scripts/lib/local-development-secrets.sh`. Do not change that default
without migrating or explicitly resetting affected local data. Valkey is started without
snapshots. Stop the foreground stack with Ctrl+C, or from another shell with
`PORT_PREFIX=271 just stop-all`; that command also stops the prefix's infrastructure processes.
Delete persistent data only with the explicit, prefix-scoped `PORT_PREFIX=271 just reset` command.

Verify the selected stack and auth contract with the actual `/health` endpoint:

```bash
curl -fsS http://localhost:27100/health
curl -fsS -o /dev/null -w '%{http_code}\n' http://localhost:27100/
```

### Cloud agents and secrets

`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, and `LINEAR_API_KEY` come from Doppler:

```bash
./scripts/init-cloud-env.sh
export CARGO_INCREMENTAL=0
# Then use the canonical command in "Local dev" above.
```

Failing GitHub auth means the token was not passed through, not that it expired:

```bash
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" <command>'
```

### Commits

- Conventional Commits (`type(scope): description`); use `chore` for `knowledge/` and `AGENTS.md`.
- Commit as the real human user. If `git config user.name`/`user.email` are missing or agent-like,
  set them from `GIT_USER_NAME`/`GIT_USER_EMAIL`; if those are absent, ask instead of committing
  with a bot identity.
- No AI attribution anywhere — commits, PRs, docs, code comments. The sole exception is yolop's
  standard `Co-Authored-By` trailer and `Generated with yolop` PR footer for work yolop performed.
