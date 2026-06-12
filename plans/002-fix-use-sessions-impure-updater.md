# Plan 002: `use-sessions` SSE handler uses a pure state updater

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d94f9a8..HEAD -- apps/ui/src/hooks/use-sessions.ts`
> If this file changed since the plan was written, compare the "Current state"
> excerpt against the live code before proceeding; on a mismatch, treat it as a
> STOP condition.

## Status

- **Priority**: P1
- **Effort**: S–M
- **Risk**: MED
- **Depends on**: 001 (so `pnpm run typecheck` exists; if 001 is not done, use
  `pnpm exec tsc --noEmit` everywhere this plan says `pnpm run typecheck`)
- **Category**: bug
- **Planned at**: commit `d94f9a8`, 2026-06-12

## Why this matters

The live-events SSE handler in `use-sessions.ts` mutates a ref
(`eventIdsRef.current`) **inside** the `setEvents` state updater. React state
updaters must be pure — React may invoke an updater more than once for a single
update (StrictMode in dev, and bailout/replay under concurrent rendering).
Each extra invocation re-runs the ref reassignment against a possibly different
`prev`, which can desynchronize the deduplication set from the events actually
in memory. The observable symptoms are duplicated or dropped chat events after
the in-memory buffer exceeds its cap. The fix is to keep the updater pure and
reconcile the dedup set in a dedicated effect.

## Current state

`apps/ui/src/hooks/use-sessions.ts`, inside the SSE `useEffect`, the per-event
listener (lines ~475–498). The dedup set `eventIdsRef` and the cap
`MAX_EVENTS_IN_MEMORY` are defined earlier in the same hook.

```tsx
      for (const eventType of SSE_EVENT_TYPES) {
        eventSource.addEventListener(eventType, (messageEvent) => {
          try {
            const event: Event = JSON.parse(messageEvent.data);

            if (eventIdsRef.current.has(event.id)) return;

            eventIdsRef.current.add(event.id);
            lastEventIdRef.current = event.id;
            setEvents((prev) => {
              const next = [...prev, event];
              if (next.length > MAX_EVENTS_IN_MEMORY) {
                // Trim oldest events and their IDs from the dedup set
                const trimmed = next.slice(next.length - MAX_EVENTS_IN_MEMORY);
                const keptIds = new Set(trimmed.map((e) => e.id));
                eventIdsRef.current = keptIds;        // <-- impure: ref write in updater
                return trimmed;
              }
              return next;
            });
          } catch (e) {
            console.error("Failed to parse SSE event:", e);
          }
        });
      }
```

The same file already follows the project's hook conventions: `useRef` for
mutable bookkeeping, `useEffect` cleanup that flips a `cancelled` flag and tears
down the event source. Match that style.

## Commands you will need

| Purpose   | Command (run from `apps/ui/`)              | Expected on success   |
|-----------|-------------------------------------------|-----------------------|
| Install   | `pnpm install`                            | exit 0                |
| Typecheck | `pnpm run typecheck` (or `pnpm exec tsc --noEmit`) | exit 0, no errors |
| Tests     | `pnpm test -- use-sessions`               | all pass              |
| Lint      | `pnpm run lint`                           | exit 0                |
| Format    | `pnpm run format:check`                   | exit 0                |

## Scope

**In scope** (the only files you should modify):
- `apps/ui/src/hooks/use-sessions.ts` — the SSE updater and one new effect.
- `apps/ui/src/__tests__/use-sessions.test.tsx` (create) — see Test plan.

**Out of scope** (do NOT touch):
- The REST initial-fetch path (the `fetchInitialEvents` effect) and
  `loadOlderEvents` — they add to `eventIdsRef` outside an updater, which is
  fine; do not change them.
- `MAX_EVENTS_IN_MEMORY`, `SSE_EVENT_TYPES`, or the reconnect logic.
- Any other hook or component.

## Git workflow

- Branch: `advisor/002-fix-use-sessions-impure-updater`.
- Conventional Commits: e.g. `fix(ui): keep use-sessions SSE updater pure`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Make the `setEvents` updater pure

Remove the ref write from inside the updater. The updater should only compute
and return the next array:

```tsx
            eventIdsRef.current.add(event.id);
            lastEventIdRef.current = event.id;
            setEvents((prev) => {
              const next = [...prev, event];
              return next.length > MAX_EVENTS_IN_MEMORY
                ? next.slice(next.length - MAX_EVENTS_IN_MEMORY)
                : next;
            });
```

**Verify**: `pnpm run typecheck` → exit 0.

### Step 2: Reconcile the dedup set in a dedicated effect

Add an effect (place it near the other effects in the hook, after the SSE
effect) that bounds `eventIdsRef` to the events still in memory. The guard
makes it a no-op for normal appends (where the set size already equals the
event count) and only rebuilds after a trim has happened:

```tsx
  // Keep the dedup set bounded to the events still in memory. Doing this in a
  // dedicated effect (not inside the setEvents updater) keeps the updater pure:
  // React may run updaters more than once under StrictMode/concurrent mode,
  // and a ref write inside one would desync the set. The size guard makes this
  // a no-op except right after the buffer is trimmed.
  useEffect(() => {
    if (eventIdsRef.current.size > events.length) {
      eventIdsRef.current = new Set(events.map((e) => e.id));
    }
  }, [events]);
```

This relies on `events` being state in the hook (it is — `setEvents` is its
setter). Confirm `events` is in scope where you add the effect.

**Verify**: `pnpm run typecheck` → exit 0; `pnpm run lint` → exit 0 (no
exhaustive-deps warnings).

### Step 3: Confirm no remaining ref writes inside updaters

**Verify**: `grep -n "eventIdsRef.current =" apps/ui/src/hooks/use-sessions.ts`
returns only the line inside the new `useEffect` from Step 2 — not inside any
`setEvents(...)` callback.

## Test plan

Create `apps/ui/src/__tests__/use-sessions.test.tsx`. Use
`renderHook` from `@testing-library/react`. Model the mocking style on an
existing test in `apps/ui/src/__tests__/` that uses `jest.mock(...)` for an
`@/lib/api/*` module (e.g. look at how `session-detail-model.test.tsx` or
`recent-sessions.test.tsx` set up their mocks, and follow the same pattern).

Mock the event-stream creation so the test can push synthetic SSE events:

- Mock the module that exports `createEventStream` (the SSE factory used by the
  hook — find it via `grep -n "createEventStream" apps/ui/src/hooks/use-sessions.ts`
  and mock that import) to return a fake `EventSource`-like object whose
  `addEventListener` handlers you can capture and invoke manually.
- Mock the REST events API the hook calls on mount so the initial fetch
  resolves with an empty list (so the test exercises only the SSE path).

Cases to cover:

1. **Dedup**: dispatch two SSE events with the same `id`; assert the hook's
   `events` contains it exactly once.
2. **No duplicates after trim**: dispatch more than `MAX_EVENTS_IN_MEMORY`
   distinct events, then re-dispatch the `id` of an event that is still in the
   in-memory window; assert it is not appended again (length unchanged).
3. **Bounded buffer**: after dispatching `MAX_EVENTS_IN_MEMORY + N` distinct
   events, assert `events.length === MAX_EVENTS_IN_MEMORY`.

Verification: `pnpm test -- use-sessions` → all pass, including the 3 new cases.

## Done criteria

ALL must hold:

- [ ] No `eventIdsRef.current = ...` assignment appears inside any `setEvents`
      updater callback (verified by Step 3 grep).
- [ ] `pnpm run typecheck` exits 0.
- [ ] `pnpm run lint` exits 0 (no new react-hooks warnings).
- [ ] `pnpm test -- use-sessions` exits 0 with the 3 new cases present and
      passing.
- [ ] No files outside the in-scope list are modified (`git status`).
- [ ] `plans/README.md` status row updated.

## STOP conditions

Stop and report back (do not improvise) if:

- The "Current state" excerpt does not match the live code (the file drifted).
- `events` is not accessible as hook state where Step 2's effect must go (the
  hook has been refactored) — report the new shape.
- Mocking the SSE stream proves intractable within the existing test harness.
  In that case, STOP and propose extracting the append+trim+dedup logic into a
  small pure helper (e.g. `applyEvent(prev, event, ids, max)`) that can be unit
  tested directly — but do not perform that refactor without sign-off.

## Maintenance notes

- If a future change reintroduces per-event side effects, keep them out of the
  `setEvents` updater — put them before/after the call or in an effect.
- A reviewer should check that the Step 2 effect's dependency array is exactly
  `[events]` and that it does not cause an extra render loop (it only writes a
  ref, never calls a setState).
- If `MAX_EVENTS_IN_MEMORY` is ever raised substantially, revisit whether the
  full-`Set` rebuild on trim needs to become incremental.
