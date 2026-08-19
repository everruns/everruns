# Maintenance surfaces

Goals and good evidence per surface. Use judgment about which ones the current task touches.

## Dependency health

Every workspace crate and pnpm package — CLI, server, worker, integrations, UI, docs — runs on
current versions, with major bumps applied rather than deferred indefinitely.

- Run `cargo outdated` (or `cargo search` per crate) and `pnpm outdated` during any
  release-readiness or dependency-scoped pass, even with no advisory outstanding. `knowledge/project/maintenance.md`
  records the current `cargo outdated` tooling caveat and the pnpm release-age floor.
- Apply and test major upgrades when the migration path is clear; document the blocker when it is not.
- Flag deprecated crates/packages with a replacement, and check for unused dependencies
  (`cargo udeps` or manual review).
- Update lockfiles intentionally.

## Knowledge and docs alignment

Docs describe current intent and constraints without drifting into code duplication: changed
behavior reflected in `knowledge/`, `apps/docs/`, OpenAPI, or examples; stale or duplicated
knowledge replaced by links to source. Follow the maintenance rules in
`knowledge/knowledge-contract.md` and `knowledge/project/maintenance.md`.

## Feature completeness across surfaces

Features that look shipped in one surface are connected and consistent in the others:

- UI affordances call real backend APIs and handle loading, errors, auth, and refresh
- backend features intended for agents or automation are exposed through MCP/app surfaces
- CLI commands and flags match current API semantics
- docs, knowledge, examples, tests, and manual test cases do not claim behavior the product lacks

Fix locally, or record the missing surface, user impact, and next action.

## Security and threat posture

New or changed attack surface is understood and its mitigations match reality: threat model updated
when trust boundaries moved, and obvious gaps in auth, validation, secret handling, or data exposure
reviewed. Check the GitHub [security overview](https://github.com/everruns/everruns/security),
[Dependabot alerts](https://github.com/everruns/everruns/security/dependabot), and
[secret scanning](https://github.com/everruns/everruns/security/secret-scanning?query=is%3Aopen+results%3Ageneric).

DeepSec runs when the pass covers security posture, release readiness, auth/tenant/public-ingress
review, or repo-wide hygiene. Setup and usage live in [`.deepsec/AGENTS.md`](../../../../.deepsec/AGENTS.md);
from `.deepsec/` after `pnpm install`:

```bash
pnpm deepsec scan --project-id everruns
pnpm deepsec process --project-id everruns --agent codex --limit <n>   # budget the AI pass
pnpm deepsec revalidate --project-id everruns --agent codex --min-severity HIGH
```

Keep the durable workspace files tracked (`.gitignore`, `AGENTS.md`, `README.md`,
`deepsec.config.ts`, `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`,
`data/*/{INFO.md,SETUP.md}`) and leave generated state (`node_modules/`, `.env*.local`,
`data/*/{files,runs,reports,project.json,tech.json}`) uncommitted unless asked. Note run IDs, scope,
finding count, and cost. File Linear issues for findings not fixed in this pass.

## Test and runtime confidence

Important paths carry the right proof, not ceremony: targeted tests for regressions, smoke tests or
manual verification where unit tests are insufficient, and checks matched to the touched surface
rather than an arbitrary full matrix.

## Performance and operational safety

Recent changes introduce no obvious scale or latency regression: query shape, pagination, indexes,
batching, and background job cost reviewed where relevant; no unbounded list paths or easy N+1s.

## Binary and artifact size

Shipped artifacts stay proportionate to what they do: release binaries and container images do not
grow silently across releases, and growth that did happen has a named cause.

Check this when the pass covers release readiness, a dependency change that pulls a new transitive
tree, or a suspicion that a binary jumped. Compare against the previous release's published tarball
before calling growth acceptable.

```bash
cargo build --release -p everruns-cli   # or -p everruns-server -p everruns-worker
ls -l target/release/everruns
```

For attribution, [`cargo-bsize`](https://github.com/Boshen/cargo-bsize) ranks a binary by crate,
generic family, duplicate dependency version, and duplicated constant:

```bash
cargo install cargo-bsize --locked
cargo bsize --bin everruns      # add --baseline <path> to diff against an earlier build
```

Know its limits before trusting the report:

- it builds `release` plus debuginfo into its own `target/bsize` rather than reusing
  `target/release`, costing several minutes and ~2 GB of disk
- `--mono`, `--macros`, and `--remarks` need nightly `-Z` flags, which the pinned stable toolchain in
  `rust-toolchain.toml` cannot provide
- its "reached by no reference the graph can see" figure covers most of the binary and is a limit of
  its call-graph analysis, not dead code — do not report it as recoverable
- it is pre-1.0 and used ad hoc, deliberately not wired into `just` or CI, and its report asks the
  reader for source-level changes only, so profile and link-level wins are its blind spot

Findings that repay the run: duplicate crate versions shipping two copies of a tree, two backends
linked for one job, and generic families instantiated per caller where one concrete function would
do. Anything too large to fix in the pass becomes a Linear issue.

## Technical debt

Structural debt is named and tracked before it compounds: god objects, duplicated logic, and
boilerplate identified with file locations and line counts; severity judged by active harm versus
friction; large non-test files (>2K lines) catalogued with the structural reason they grew; hacks
and open vulnerabilities surfaced with code references. Each finding becomes an actionable Linear
issue.

## Issue tracking hygiene

Linear reflects reality closely enough that stalled work is visible. Review OSS project issues
already in `In Progress`; treat `updatedAt` older than 1 day as stale and triage, comment, re-scope,
or move them out. Capture maintenance findings you are not fixing as issues rather than leaving them
implicit.

## Repo workflow hygiene

Agent instructions, commands, and skills still match reality and do not contradict each other.

- Each fact belongs to exactly one layer — see the guidance-layering table in
  [`AGENTS.md`](../../../../AGENTS.md). Prune restatements instead of syncing them; move detail a
  skill only needs sometimes into a `references/` file.
- Release and maintenance instructions point at the canonical workflow rather than duplicating it.
- Check plugin surfaces against recent upstream platform changes across
  `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`,
  `.cursor-plugin/marketplace.json`, and the per-host manifests under `plugins/everruns-dev/`
  (see [`plugins/AGENTS.md`](../../../../plugins/AGENTS.md)); prove parity with
  `scripts/test-everruns-dev-plugin.sh`.
