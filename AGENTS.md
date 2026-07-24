## Everruns

Agentic runtime and control plane: Rust workspace in `crates/` (edition 2024), Next.js UI in
`apps/ui/`, docs site in `apps/docs/` (content in `docs/`), embeddable examples in `examples/`.
`just --list` shows every dev command.

### Where guidance lives

Keep each fact in exactly one layer, and read the layer that owns it.

| Layer | Owns |
|---|---|
| `AGENTS.md` (this file, plus per-subtree ones) | repo gotchas that are not discoverable from the code |
| `specs/` | why and what: design intent, contracts, success bars — start at `specs/README.md` |
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
- Run `just pre-push` before pushing.
- Specs capture why/what; link to source instead of copying fields, enum variants, SQL DDL, or API
  shapes. `docs/` holds public product documentation only — proposals and investigations belong in
  `specs/` or `proposals/`.
- Linear: OSS project, EVE team.

### Local dev

```bash
PORT_PREFIX=271 just start-dev   # in-memory, no external services
PORT_PREFIX=271 just start-all   # PostgreSQL + Valkey + NATS
cd apps/ui && ./node_modules/.bin/next dev --port 9120   # UI only
```

Use a distinct `PORT_PREFIX` per worktree so parallel stacks do not collide.

### Cloud agents and secrets

`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, and `LINEAR_API_KEY` come from Doppler:

```bash
./scripts/init-cloud-env.sh
export CARGO_INCREMENTAL=0
doppler run -- just start-dev --no-watch
```

Failing GitHub auth means the token was not passed through, not that it expired:

```bash
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" <command>'
```

### Commits

- Conventional Commits (`type(scope): description`); use `chore` for `specs/` and `AGENTS.md`.
- Commit as the real human user. If `git config user.name`/`user.email` are missing or agent-like,
  set them from `GIT_USER_NAME`/`GIT_USER_EMAIL`; if those are absent, ask instead of committing
  with a bot identity.
- No AI attribution anywhere — commits, PRs, docs, code comments. The sole exception is yolop's
  standard `Co-Authored-By` trailer and `Generated with yolop` PR footer for work yolop performed.
