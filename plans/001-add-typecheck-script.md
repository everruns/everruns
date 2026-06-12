# Plan 001: `apps/ui` has a `typecheck` script wired into CI

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d94f9a8..HEAD -- apps/ui/package.json .github/workflows/ci.yml`
> If either in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `d94f9a8`, 2026-06-12

## Why this matters

`apps/ui` is TypeScript with `strict: true` and `noEmit: true`, but there is no
script that runs the type checker. Type errors are only surfaced by `next build`
(slow, and only on the parts of the graph the build reaches) or by an editor.
CI's `ui-build` job runs format-check, lint, and tests but never `tsc`, so a PR
that breaks types in code paths the tests don't import can go green. Adding a
`typecheck` script and a CI step closes that gap with a fast, deterministic
gate.

## Current state

- `apps/ui/package.json` — the UI package manifest. Its `scripts` block today
  (verify it matches before editing):

  ```json
  "scripts": {
    "dev": "next dev --turbopack --port 9100",
    "build": "next build --turbopack",
    "start": "next start",
    "lint": "oxlint -c oxlint.json ./src",
    "format": "oxfmt --write ./src",
    "format:check": "oxfmt --check ./src",
    "test": "jest",
    "test:watch": "jest --watch",
    "e2e": "playwright test",
    "e2e:ui": "playwright test --ui",
    "e2e:headed": "playwright test --headed"
  },
  ```

- `apps/ui/tsconfig.json` already sets `"noEmit": true` and `"strict": true`,
  so `tsc` is safe to run as a pure checker (it emits nothing).
- `typescript` is already a dev dependency of the package (it builds with Next),
  so `tsc` is resolvable via `pnpm exec tsc` — no new dependency is needed.
  Confirm with the install + typecheck commands below before editing CI.
- `.github/workflows/ci.yml` — the `ui-build` job runs with
  `working-directory: apps/ui`. Its steps today (lines ~1078–1085):

  ```yaml
      - name: Format check
        run: pnpm run format:check

      - name: Lint
        run: pnpm run lint

      - name: Run tests
        run: pnpm test
  ```

## Commands you will need

| Purpose   | Command (run from `apps/ui/`)        | Expected on success      |
|-----------|--------------------------------------|--------------------------|
| Install   | `pnpm install`                       | exit 0                   |
| Typecheck | `pnpm exec tsc --noEmit`             | exit 0, no errors        |
| Lint      | `pnpm run lint`                      | exit 0                   |
| Format    | `pnpm run format:check`              | exit 0                   |

## Scope

**In scope** (the only files you should modify):
- `apps/ui/package.json` — add one script entry.
- `.github/workflows/ci.yml` — add one step to the `ui-build` job.

**Out of scope** (do NOT touch, even though they look related):
- `scripts/lib/pre-push.sh` — adding a UI-typecheck step there requires
  renumbering its `N/9` step labels and is deferred (see Maintenance notes).
  Do not modify it in this plan.
- `apps/ui/tsconfig.json` — it is already correct; do not change compiler
  options. If `pnpm exec tsc --noEmit` reports pre-existing type errors, that is
  a STOP condition (see below) — do not "fix" tsconfig to silence them.

## Git workflow

- Branch: `advisor/001-add-typecheck-script` (or the repo's branch convention).
- Conventional Commits (see `git log`): e.g.
  `chore(ui): add typecheck script and wire into CI`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Confirm the type checker passes on the current tree

From `apps/ui/`, run `pnpm install` then `pnpm exec tsc --noEmit`.

**Verify**: `pnpm exec tsc --noEmit` → exit 0, no errors printed.

If it reports errors, STOP (see STOP conditions) — this plan adds a gate, it
does not fix a backlog of type errors.

### Step 2: Add the `typecheck` script

In `apps/ui/package.json`, add a `"typecheck"` entry to `scripts`. Place it
immediately after the `"format:check"` line:

```json
    "format:check": "oxfmt --check ./src",
    "typecheck": "tsc --noEmit",
```

**Verify**: `pnpm run typecheck` → exit 0, no errors.

### Step 3: Add the CI step

In `.github/workflows/ci.yml`, in the `ui-build` job, add a `Type check` step
between the existing `Lint` and `Run tests` steps:

```yaml
      - name: Lint
        run: pnpm run lint

      - name: Type check
        run: pnpm run typecheck

      - name: Run tests
        run: pnpm test
```

**Verify**: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"` → exit 0 (the workflow is still valid YAML). If `python3`/`yaml` is unavailable, instead run any installed YAML linter, or visually confirm indentation matches the surrounding steps exactly (six spaces before `- name:`).

## Test plan

No unit tests apply (this is tooling). Verification is the `typecheck` script
itself plus the YAML validity check above. There is nothing to add to the jest
suite.

## Done criteria

ALL must hold:

- [ ] `apps/ui/package.json` has `"typecheck": "tsc --noEmit"` in `scripts`.
- [ ] From `apps/ui/`, `pnpm run typecheck` exits 0.
- [ ] `.github/workflows/ci.yml` `ui-build` job has a `Type check` step running
      `pnpm run typecheck`, between `Lint` and `Run tests`.
- [ ] The workflow file is valid YAML.
- [ ] No files outside the in-scope list are modified (`git status`).
- [ ] `plans/README.md` status row updated.

## STOP conditions

Stop and report back (do not improvise) if:

- `pnpm exec tsc --noEmit` reports one or more pre-existing type errors on the
  unmodified tree. Report the count and the first few errors — fixing them is a
  separate effort, not part of adding the gate.
- The `scripts` block in `package.json` does not match the "Current state"
  excerpt (the manifest has drifted).
- A `typecheck` script already exists — report what it runs instead of
  overwriting it.

## Maintenance notes

- Follow-up (deliberately deferred): add an equivalent UI-typecheck step to
  `scripts/lib/pre-push.sh`, mirroring the existing `4/9 UI formatting` and
  `5/9 UI linting` blocks (guarded by `if [ -d ".../apps/ui/node_modules" ]`).
  This requires renumbering the `N/9` labels, so it is intentionally out of
  scope here to keep this change low-risk.
- A reviewer should confirm the CI step runs only when `apps/ui/**` changes —
  it inherits the job's existing `needs.changes.outputs.ui == 'true'` guard, so
  no extra gating is needed.
