# Plan 004: Test coverage for the A2UI renderer (untrusted LLM content)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d94f9a8..HEAD -- apps/ui/src/components/chat/a2ui-renderer.tsx apps/ui/src/lib/a2ui-utils.ts`
> If either changed, compare the "Current state" excerpts against the live code
> before proceeding; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW (adds tests only; no source change)
- **Depends on**: none (do 001 first only if you want `pnpm run typecheck`)
- **Category**: tests
- **Planned at**: commit `d94f9a8`, 2026-06-12

## Why this matters

`A2UIBlock` renders **LLM-generated JSON** into live React UI inside chat. It is
750 lines, handles partial/streaming JSON, has a security-critical URL allowlist
(`isSafeUrl`, tagged `THREAT[TM-WEB-A2UI-01]`), an error boundary, and
interactive form/action dispatch — and it has **zero tests**. Any change to the
parser, the URL guard, or the node switch can silently regress rendering or the
security boundary. This plan adds a focused test suite that locks in the
load-bearing behaviors, especially the URL allowlist and the partial-parse
fallbacks.

This plan is **tests only** — it does not modify `a2ui-renderer.tsx`. If a test
reveals a real bug, STOP and report it (see STOP conditions); fixing it is a
separate plan.

## Current state

`apps/ui/src/components/chat/a2ui-renderer.tsx` public API:

```tsx
interface A2UIBlockProps {
  code: string;          // Raw A2UI JSON (without the ```a2ui fences)
  isStreaming?: boolean; // Whether the LLM is still streaming this content
}

export function A2UIBlock({ code, isStreaming }: A2UIBlockProps) { /* ... */ }
```

Behaviors to pin down (all read from the current source):

- **Parsing** (`A2UIBlock` → `parseA2UI` from `@/lib/a2ui-utils`): valid JSON
  parses; malformed JSON falls back to a best-effort partial parse, else `null`.
- **Null-tree fallback** (`A2UIRoot`, lines ~736–748): when the parsed tree is
  `null`/`undefined`, if `isStreaming` it renders a `…` placeholder; otherwise
  it renders the raw `code` inside a `<pre><code>`.
- **URL allowlist** (`isSafeUrl`, lines ~114–122): only `http:`, `https:`,
  `mailto:` are allowed.
  - `Image` node (lines ~342–351): `if (!src || !isSafeUrl(src)) return null;`
    — an unsafe `src` renders nothing.
  - `open_url` action dispatch (`useActionDispatch`, lines ~134–145): only calls
    `window.open(action.url, "_blank", "noopener,noreferrer")` when
    `isSafeUrl(action.url)` is true.
- **Node switch** (`renderNode`, line ~196): supported `type` values include
  `Stack`, `Card`, `Separator`, `Heading`, `Badge`, `Image`, plus form nodes.
  A `Heading` node renders text from `props.text`; a `Badge` renders
  `props.label`.

**Important rendering dependency**: `A2UIRoot` calls `useActionDispatch()`,
which calls `useSessionContext()` (from
`@/app/(main)/sessions/[sessionId]/session-context`) and `useParams()` (from
`next/navigation`). **Every** render of `A2UIBlock` runs these, so tests must
mock both or the render throws.

Test harness facts:
- Jest + `@testing-library/react`, jsdom env (`apps/ui/jest.config.js`).
- Module alias `^@/(.*)$` → `src/$1`.
- Existing sibling test to model structure on:
  `apps/ui/src/__tests__/todo-list-renderer.test.tsx` (renders a chat renderer
  component with `render`/`screen`).
- Existing utils test (already covers `splitA2UIBlocks`/parsing helpers, so do
  not duplicate those): `apps/ui/src/__tests__/a2ui-utils.test.ts`.

## Commands you will need

| Purpose   | Command (run from `apps/ui/`)              | Expected on success   |
|-----------|-------------------------------------------|-----------------------|
| Install   | `pnpm install`                            | exit 0                |
| Tests     | `pnpm test -- a2ui-renderer`              | all pass              |
| Typecheck | `pnpm run typecheck` (or `pnpm exec tsc --noEmit`) | exit 0        |
| Lint      | `pnpm run lint`                           | exit 0                |
| Format    | `pnpm run format:check`                   | exit 0                |

## Scope

**In scope** (the only file you should create):
- `apps/ui/src/__tests__/a2ui-renderer.test.tsx` (create).

**Out of scope** (do NOT touch):
- `apps/ui/src/components/chat/a2ui-renderer.tsx` — no source edits. This is a
  characterization-test plan; if a test would only pass by changing the
  component, that means you found a bug — STOP and report.
- `apps/ui/src/lib/a2ui-utils.ts` and its existing test.
- Any provider or other component.

## Git workflow

- Branch: `advisor/004-a2ui-renderer-tests`.
- Conventional Commits: e.g. `test(ui): cover A2UI renderer parsing and URL guard`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Scaffold the test file with the required mocks

Create `apps/ui/src/__tests__/a2ui-renderer.test.tsx`. Mock `next/navigation`
and the session context so `useActionDispatch` can run. Capture the dispatch
target by spying on the session context's `sendMessage.mutate` and on
`window.open`.

Sketch (adapt names to the actual exports — verify
`useSessionContext`'s shape and the session-context module path with
`grep -n "export function useSessionContext\|sendMessage" apps/ui/src/app/\(main\)/sessions/\[sessionId\]/session-context.tsx`):

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { A2UIBlock } from "@/components/chat/a2ui-renderer";

const sendMessageMutate = jest.fn();

jest.mock("next/navigation", () => ({
  useParams: () => ({ sessionId: "session_test" }),
}));

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => ({ sendMessage: { mutate: sendMessageMutate } }),
}));

beforeEach(() => {
  sendMessageMutate.mockClear();
});
```

**Verify**: `pnpm test -- a2ui-renderer` runs the (empty) file without a module
resolution error (it may report "no tests" — that is fine at this step).

### Step 2: Cover structured rendering

Add tests that render valid JSON and assert output:

- A `Heading` (`{"type":"Heading","props":{"text":"Hello"}}`) renders the text
  "Hello".
- A `Stack` containing a `Badge` (`props.label`) renders the label text.

**Verify**: `pnpm test -- a2ui-renderer` → these pass.

### Step 3: Cover the URL allowlist (security boundary)

- **Image, safe URL**: `{"type":"Image","props":{"src":"https://example.com/a.png","alt":"x"}}`
  renders an `<img>` whose `src` is the https URL (`screen.getByRole("img")`).
- **Image, unsafe URL**: same node with `"src":"javascript:alert(1)"` (and again
  with `"src":"data:text/html,x"`) renders **no** `<img>`
  (`screen.queryByRole("img")` is null).
- **open_url action, unsafe URL**: render a node with an `open_url` action whose
  `url` is `javascript:alert(1)`; spy on `window.open`
  (`jest.spyOn(window, "open").mockImplementation(() => null)`); trigger the
  action (click the rendered control) and assert `window.open` was **not**
  called. Determine the exact node/action shape that produces a clickable
  `open_url` from the `renderNode` switch (search the source for `open_url` and
  the node type that carries an `onClick`/action) before writing this case.
- **open_url action, safe URL**: same with `https://example.com`; assert
  `window.open` **was** called once.

**Verify**: `pnpm test -- a2ui-renderer` → these pass. If the unsafe-URL image
case fails because an `<img>` IS rendered, that is a real security regression —
STOP and report.

### Step 4: Cover parse fallbacks

- **Malformed JSON, not streaming** (`isStreaming` omitted/false): pass
  `code="{ not json"`; assert the raw code text is shown (the `<pre><code>`
  fallback) — e.g. `screen.getByText(/not json/)`.
- **Malformed JSON, streaming** (`isStreaming`): pass the same code with
  `isStreaming`; assert the streaming placeholder (`…`) is shown and the raw
  fallback is NOT.
- **Empty code**: `code=""` renders nothing (component returns `null`); assert
  `container.firstChild` is null.

**Verify**: `pnpm test -- a2ui-renderer` → these pass.

### Step 5: Full verification

**Verify** (from `apps/ui/`): `pnpm test -- a2ui-renderer` all pass,
`pnpm run lint` exit 0, `pnpm run format:check` exit 0, `pnpm run typecheck`
exit 0.

## Test plan

Summarized (the steps above are the test plan):
- Structured rendering: Heading text, Badge label inside Stack.
- URL allowlist: Image safe vs `javascript:`/`data:` (no img); `open_url` action
  safe (calls `window.open`) vs unsafe (does not).
- Parse fallbacks: malformed not-streaming → raw code; malformed streaming →
  `…` placeholder; empty → null.
- Structural pattern: model on `apps/ui/src/__tests__/todo-list-renderer.test.tsx`.
- Target: a new `a2ui-renderer.test.tsx` with ~8–10 assertions across these.

## Done criteria

ALL must hold:

- [ ] `apps/ui/src/__tests__/a2ui-renderer.test.tsx` exists.
- [ ] `pnpm test -- a2ui-renderer` exits 0 and includes the cases from Steps
      2–4 (structured render, URL allowlist for both Image and `open_url`, and
      the three parse-fallback cases).
- [ ] `pnpm run typecheck`, `pnpm run lint`, `pnpm run format:check` all exit 0.
- [ ] `a2ui-renderer.tsx` is unchanged (`git status` shows only the new test
      file added).
- [ ] `plans/README.md` status row updated.

## STOP conditions

Stop and report back (do not improvise) if:

- The unsafe-URL Image case renders an `<img>`, or the unsafe `open_url` case
  calls `window.open` — that is a real security bug; report it, do not "fix" the
  test to match the buggy behavior.
- `useSessionContext`'s real shape differs enough that the mock can't satisfy
  `useActionDispatch` — report the actual shape from the source.
- Rendering `A2UIBlock` requires additional providers (e.g. a theme/locale
  provider) that aren't obvious — report which, rather than guessing.
- You cannot determine from the source which node/action shape produces a
  clickable `open_url` — report it instead of inventing one.

## Maintenance notes

- These are characterization tests: they encode current behavior, especially
  the `THREAT[TM-WEB-A2UI-01]` URL allowlist. A reviewer changing `isSafeUrl` or
  the `Image`/`open_url` paths should expect these tests to change deliberately,
  never silently.
- If new A2UI node types are added to `renderNode`, add a minimal render
  assertion here for each so the switch stays covered.
