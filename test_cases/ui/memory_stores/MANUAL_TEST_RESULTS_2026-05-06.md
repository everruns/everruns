# Manual UI Test Results — memory_stores — 2026-05-06

## Environment

- **Auth Mode**: dev (in-memory storage, anonymous user, soft auth)
- **Stack**: `just start-dev --no-watch` (server + in-process worker + Caddy at :27100, Next.js dev at :27105)
- **PORT_PREFIX**: 271
- **Build**: `target/debug/everruns-server` (everruns 0.8.26)
- **UI**: `apps/ui` Next.js 16.2.4 (Turbopack)
- **Browser**: agent-browser (Chromium)

## Test Summary

| Test | Result | Notes |
|------|--------|-------|
| TC001 Create Memory Store | PASS | New store auto-selected, `mst_<32-hex>` ID, `0 active memories` shown |
| TC002 Create Default Memory Store | PASS | Default badge appears on new store; previously default cleared |
| TC003 Duplicate Store Name Validation | PARTIAL | Server returns the correct 409 with body `memory store name already exists`; the UI does not surface this error to the user — the failed mutation is only visible in the Next.js dev "1 issue" overlay. Dialog stays open and the duplicate is not created. |
| TC004 Search and Filter Memories | NOT RUN | Requires seeded memories (agent `remember` calls). Out of scope for a smoke pass without provider keys configured. |
| TC005 Forget a Memory | NOT RUN | Same as TC004. |
| TC006 Capability Config Store Picker | NOT RUN | `/agents/new` and `/agents` redirect to `/login` in dev mode (anonymous-user flow doesn't satisfy the agents page auth guard). Memory page itself works because its layout tolerates the in-flight auth state. Needs `just start-all` with a real signed-in user. |
| TC007 Cross-Org Memory Store Isolation | NOT RUN | Requires multi-org auth (full mode). |
| TC008 Memory Page Empty State | PASS | Brain icon, "No memory stores" heading, explanatory text, and CTA all render; two-column layout is suppressed. |

## Detailed Notes

### TC001 — PASS
Empty-state CTA opens the dialog, name `team-knowledge` accepted, **Create** transitions back to the populated layout. Card shows `mst_019dfeb5d6f976d3ba1463b4fdd290e2`, `0 active memories`, no Default badge. Right pane shows the "No memories yet…" empty state. Screenshot: `/tmp/memory_after_create.png`.

### TC002 — PASS
Created `org-default` with the Default checkbox ticked. Card now shows the **Default** star badge; `team-knowledge` no longer has one. Both stores listed in the sidebar. Screenshot: `/tmp/memory_default_created.png`.

### TC003 — PARTIAL (UI gap)
Submitting `TEAM-KNOWLEDGE` (existing name in different casing) hits the server, which correctly returns `memory store name already exists`. The dialog stays open with the typed value, and the store list is unchanged (confirming the server-side uniqueness check). However, no toast / inline error is rendered for the user — the failure is only visible because Next.js dev surfaces the unhandled `ApiError` in the "1 issue" overlay. In production builds this would be silent. Worth a follow-up to wrap `useCreateMemoryStore` with toast-on-error in `apps/ui/src/app/(main)/memory-stores/page.tsx`. Screenshot: `/tmp/dup_error_overlay.png`.

### TC008 — PASS
Fresh dev DB has zero stores and `/memory-stores` renders the centered empty state with the brain icon, "No memory stores" heading, paragraph mentioning the auto-default-on-first-use, and the **New Store** call-to-action. Two-column layout is correctly suppressed. Screenshot: `/tmp/memory_empty.png`.

## Issues Found

### Issue #1 (Low): Duplicate-name error not surfaced to user
- **Severity**: Low (server-side enforcement is correct, no data integrity risk)
- **Steps**: Create store with a name; open New Store dialog again and re-enter the same name (any casing); click Create.
- **Expected**: A user-visible toast / inline error such as "A memory store with that name already exists." Dialog stays open.
- **Actual**: API correctly returns 409 `memory store name already exists`, but the UI swallows the rejection silently. Only Next.js dev mode shows it via the issues overlay; in production the user would see no feedback at all.
- **Fix sketch**: Wire `onError` on `useCreateMemoryStore` (or surround `mutateAsync` in the dialog) to render a toast / inline message; clear it on successful create.
- **File**: `apps/ui/src/app/(main)/memory-stores/page.tsx` (`CreateStoreDialog`).

## Skipped — environmental gaps

- TC004 / TC005 need seeded memories. The minimum to seed is a running agent that uses the `memory` capability with provider keys configured. The dev stack doesn't expose a "manual remember" UI, so a seed script or agent run is required.
- TC006 / TC007 need a real signed-in user (full auth mode). The dev anonymous-user flow loads `/memory-stores` but bounces `/agents*` to `/login`; the agent capability-config picker therefore can't be reached without `just start-all` plus a login.

These test cases are documented and ready to run against a `just start-all` deployment; they're listed here as not-run rather than failed.
