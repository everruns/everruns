# Plan 003: Remove the `radix-ui` dependency (migrate Collapsible to Base UI)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d94f9a8..HEAD -- apps/ui/src/components/ui/collapsible.tsx apps/ui/package.json`
> If either in-scope file changed, compare the "Current state" excerpts against
> the live code before proceeding; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: MED
- **Depends on**: none (independent; do 001 first only if you want
  `pnpm run typecheck` as a verification command)
- **Category**: tech-debt
- **Planned at**: commit `d94f9a8`, 2026-06-12

## Why this matters

The UI standardized its primitive layer on **Base UI** (`@base-ui/react`,
imported by 15 `src/components/ui/*` files). Exactly one component still pulls in
a whole separate primitive library — `radix-ui` — and it is used only by
`src/components/ui/collapsible.tsx`. Migrating that one file to Base UI's
Collapsible lets us drop the entire `radix-ui` dependency, removing a redundant
UI library from the bundle and eliminating a second set of primitive APIs
contributors must reason about.

> Note: `@openuidev/react-ui` is **not** in scope — it is a distinct feature
> (OpenUI block rendering in chat), not a redundant primitive library. Leave it.

## Current state

`apps/ui/src/components/ui/collapsible.tsx` (entire file, on `radix-ui`):

```tsx
"use client";

import { Collapsible as CollapsiblePrimitive } from "radix-ui";

function Collapsible({ ...props }: React.ComponentProps<typeof CollapsiblePrimitive.Root>) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

function CollapsibleTrigger({
  ...props
}: React.ComponentProps<typeof CollapsiblePrimitive.CollapsibleTrigger>) {
  return <CollapsiblePrimitive.CollapsibleTrigger data-slot="collapsible-trigger" {...props} />;
}

function CollapsibleContent({
  ...props
}: React.ComponentProps<typeof CollapsiblePrimitive.CollapsibleContent>) {
  return <CollapsiblePrimitive.CollapsibleContent data-slot="collapsible-content" {...props} />;
}

export { Collapsible, CollapsibleTrigger, CollapsibleContent };
```

**Key fact for this migration**: Base UI's Collapsible parts are
`Collapsible.Root`, `Collapsible.Trigger`, and `Collapsible.Panel` — Base UI
calls the content wrapper **`Panel`**, whereas radix calls it `Content`. The
public exports of this wrapper file (`Collapsible`, `CollapsibleTrigger`,
`CollapsibleContent`) **must stay named exactly the same** so consumers don't
change; only the underlying primitive changes.

Exemplar of the repo's Base UI wrapper convention —
`apps/ui/src/components/ui/dialog.tsx`:

```tsx
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
// ...
function Dialog({ ...props }: DialogPrimitive.Root.Props) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}
```

Note the Base UI conventions to match: subpath import
(`@base-ui/react/<component>`), and namespaced prop types (e.g.
`DialogPrimitive.Root.Props`) rather than `React.ComponentProps<typeof ...>`.

Consumers (must keep working unchanged — confirm they still import the same
names after the change):
- `apps/ui/src/components/agents/selected-capability-list.tsx` (uses
  `Collapsible`, `CollapsibleTrigger`, `CollapsibleContent`)
- `apps/ui/src/components/llm/llm-history-viewer.tsx`
- `apps/ui/src/components/ai-elements/file-tree.tsx`

`@base-ui/react` is already a dependency (pinned to `1.5.0` in the lockfile)
and ships a `./collapsible` subpath, so **no new dependency is added** — one is
removed.

## Commands you will need

| Purpose   | Command (run from `apps/ui/`)              | Expected on success   |
|-----------|-------------------------------------------|-----------------------|
| Install   | `pnpm install`                            | exit 0                |
| Typecheck | `pnpm run typecheck` (or `pnpm exec tsc --noEmit`) | exit 0, no errors |
| Tests     | `pnpm test`                               | all pass              |
| Lint      | `pnpm run lint`                           | exit 0                |
| Format    | `pnpm run format:check`                   | exit 0                |
| Build     | `pnpm run build`                          | exit 0                |

## Suggested executor toolkit

- Base UI Collapsible reference: https://base-ui.com/react/components/collapsible
  — read the "Anatomy" section to confirm the `Root` / `Trigger` / `Panel`
  parts and their prop names before editing.

## Scope

**In scope** (the only files you should modify):
- `apps/ui/src/components/ui/collapsible.tsx` — rewrite on Base UI.
- `apps/ui/package.json` — remove the `radix-ui` dependency line.
- `apps/ui/pnpm-lock.yaml` — updated automatically by `pnpm install`; commit the
  result. Do not hand-edit it.

**Out of scope** (do NOT touch):
- The three consumer files above — they must keep working without edits. If any
  consumer turns out to pass a radix-only prop (e.g. `forceMount`) to
  `CollapsibleContent`, that is a STOP condition (none was observed).
- `@openuidev/*` and any other dependency.
- Any other `src/components/ui/*` wrapper.

## Git workflow

- Branch: `advisor/003-drop-radix-ui-collapsible`.
- Conventional Commits: e.g.
  `refactor(ui): migrate Collapsible to Base UI and drop radix-ui`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Rewrite `collapsible.tsx` on Base UI

Replace the file contents, keeping the exported names identical and mapping
radix `Content` → Base UI `Panel`. Match the `dialog.tsx` convention (subpath
import, namespaced prop types):

```tsx
"use client";

import { Collapsible as CollapsiblePrimitive } from "@base-ui/react/collapsible";

function Collapsible({ ...props }: CollapsiblePrimitive.Root.Props) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

function CollapsibleTrigger({ ...props }: CollapsiblePrimitive.Trigger.Props) {
  return <CollapsiblePrimitive.Trigger data-slot="collapsible-trigger" {...props} />;
}

function CollapsibleContent({ ...props }: CollapsiblePrimitive.Panel.Props) {
  return <CollapsiblePrimitive.Panel data-slot="collapsible-content" {...props} />;
}

export { Collapsible, CollapsibleTrigger, CollapsibleContent };
```

If the exact namespaced prop-type names differ in `@base-ui/react@1.5.0` (the
typecheck will tell you), use whatever the package exports for the Root/Trigger/
Panel props, following the `dialog.tsx` pattern. Keep the `data-slot` attribute
values exactly as shown (existing CSS/selectors may rely on
`collapsible-content`).

**Verify**: `pnpm run typecheck` → exit 0.

### Step 2: Remove the `radix-ui` dependency

In `apps/ui/package.json`, delete the dependency line:

```json
    "radix-ui": "^1.4.3",
```

Then run `pnpm install` to update the lockfile.

**Verify**:
- `grep -rn "radix-ui" apps/ui/src` → **no matches**.
- `grep -n "radix-ui" apps/ui/package.json` → **no matches**.
- `pnpm install` → exit 0.

### Step 3: Full verification

**Verify** (all from `apps/ui/`):
- `pnpm run typecheck` → exit 0
- `pnpm run lint` → exit 0
- `pnpm run format:check` → exit 0
- `pnpm test` → all pass
- `pnpm run build` → exit 0

## Test plan

No new unit test is required (this is a like-for-like primitive swap behind a
stable public API). Regression coverage is:

- The existing jest suite must stay green, especially any test that renders the
  three consumer components (`file-tree`, `llm-history-viewer`,
  `selected-capability-list`). Find them with
  `grep -rl "file-tree\|llm-history-viewer\|selected-capability-list" apps/ui/src/__tests__`
  and confirm they pass.
- `pnpm run build` succeeding proves the Base UI Collapsible resolves and types
  check across all consumers.

If you want belt-and-suspenders coverage and a consumer test does not already
exercise expand/collapse, you may add one small render test for `file-tree.tsx`
that toggles a node and asserts the panel content appears — model it on an
existing component test in `apps/ui/src/__tests__/` (e.g.
`todo-list-renderer.test.tsx`). This is optional, not required for done.

## Done criteria

ALL must hold:

- [ ] `grep -rn "radix-ui" apps/ui/src` returns no matches.
- [ ] `grep -n "radix-ui" apps/ui/package.json` returns no matches.
- [ ] `collapsible.tsx` imports from `@base-ui/react/collapsible` and still
      exports `Collapsible`, `CollapsibleTrigger`, `CollapsibleContent`.
- [ ] `pnpm run typecheck`, `pnpm run lint`, `pnpm run format:check`,
      `pnpm test`, and `pnpm run build` all exit 0.
- [ ] `pnpm-lock.yaml` no longer resolves `radix-ui` (it is updated by
      `pnpm install`, not hand-edited).
- [ ] Only in-scope files modified (`git status`): `collapsible.tsx`,
      `package.json`, `pnpm-lock.yaml` (and an optional new test if you added
      one).
- [ ] `plans/README.md` status row updated.

## STOP conditions

Stop and report back (do not improvise) if:

- A consumer passes a prop to `Collapsible*` that Base UI does not support
  (e.g. radix's `forceMount`, or `asChild` semantics that differ). Report which
  consumer and which prop — the fix may require a consumer-side change that
  expands this plan's scope.
- Base UI's open/closed state mechanism differs in a way that visibly changes
  expand/collapse behavior in the three consumers (e.g. animation/measure
  differences that break layout). Report it rather than papering over it.
- `pnpm run build` fails for a reason traceable to the Collapsible swap.
- After removing `radix-ui`, anything else still imports it (the Step 2 grep
  finds matches outside `collapsible.tsx`) — do not remove the dep in that case.

## Maintenance notes

- After this lands, `radix-ui` should never reappear in `apps/ui/package.json`;
  new components use `@base-ui/react`. A reviewer should reject any new
  `from "radix-ui"` import.
- Watch the three consumers in review for any reliance on radix-specific DOM
  structure or data attributes; the `data-slot="collapsible-content"` value was
  preserved specifically to avoid CSS regressions.
