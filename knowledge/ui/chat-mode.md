---
type: Decision
title: "Page-aware Chat mode"
description: "How the Adoption-gated split shell connects permanent Platform Chat to the current page, with Agents as the first editing integration."
tags:
  - ui
  - agents
  - platform-chat
  - feature-flags
---

# Page-aware Chat mode

## Intent

Operate on the thing beside the conversation. Navigation should not manufacture
conversation turns; mouse edits and requested Chat edits should stay visible in
the same workspace. The design exploration supplies the split workspace and
change markers. The subsequent product decision also connects Chat to the actual
Agent form: adjacent-page edits stage in that existing draft and use its Save
and Discard actions. There is no separate Chat draft or per-change acceptance queue.

This is an incremental UI program. Agents is the first integration; the shell
and page registration contract must support subsequent entity pages without
duplicating the transcript, composer, or navigation model.

## Rollout and ownership

`chat_mode` is a separate **Adoption** flag, off until the organization opts in.
It does not require or enroll `chat_threads`. Deployment availability alone must
never enable it, including during organization loading. Turning it off restores
the existing shell. The source catalog is
[`feature_flags.rs`](../../crates/server/src/records/feature_flags.rs).

The app layout owns one long-lived
[`ChatModeShell`](../../apps/ui/src/components/chat-mode/chat-mode-shell.tsx).
It reuses the person's permanent Platform Chat and existing `SessionProvider`
and `ChatPanel`. Entering via Chat or Command/Ctrl-J preserves the route and the
mounted page. Back to app hides the Chat without tearing down its stream or
composer. An organization switch destroys the shell's state. A direct `/chats`
entry under the flag opens Chat beside Agents; side-chat URLs keep working.

The desktop geometry follows the exploration: up to 600 px Chat on the left,
the current page in the middle, and a 48 px navigation rail on the right.
The normal sidebar is 240 px. Widths transition over 280 ms with ease-out;
reduced motion removes the transition. The grid stays within the viewport;
automatic transcript scrolling must target its own container rather than scrolling
ancestor containers and moving the adjacent page. On small screens, Chat takes the content
area while open and Back to app returns to the mounted page. Agents' columns
respond to the width of the page pane, not the browser viewport.

Rail destinations and visibility come from the existing navigation model,
including organization role, feature and policy restrictions. The rail must
not grow an independent page catalog. Agent settings drawers mount within the
page pane with a page-only backdrop, leaving Chat accessible.

## Page context is data

[`ChatModeContext`](../../apps/ui/src/components/chat-mode/chat-mode-context.tsx)
is an optional page adapter. Agents registers its identity, readable label,
route including tab/sheet query, and editing state. Unregistered pages still
supply their route; entity-aware registration and change marking are separate
steps in their migration.

Each send snapshots context into **request-level message metadata**. Retrying a
send retains that snapshot and the original client message id even if the user
has navigated. Authored message text stays unchanged. A quiet context label is
rendered when the page differs from the preceding user message; navigation alone
creates no input event. Metadata contains a browser draft binding and revision,
not form bodies, credential inputs, or page HTML.
Only known tab, sheet and edit-mode query parameters travel with the route;
arbitrary callback parameters are excluded.

The managed Platform Chat opts into the `page_context` field of the existing
Message Metadata model-view provider. That provider supplies bounded JSON to the
model without changing stored message text. The annotation is expressly
untrusted data: identity resolves a reference, but neither labels nor routes
grant permission or widen the user's request. Other agents retain the default
timestamp-only configuration.

Manual Save, Discard and Undo create quiet UI action notes. Pending notes travel
with the next authored message, then render with that message's transcript.
They do not start unsolicited model turns. This deliberately reuses the current
message protocol instead of inventing a second conversation transport.

## Live Agent drafts

The Agent page registers a typed form adapter with the shell. The managed
Platform Chat receives `read_page_draft` and `patch_page_draft` as ordinary
[client-side tools](../execution/client-side-tools.md), only when the organization
has adopted Chat mode. Both the gRPC and in-process worker loading paths enforce
that gate. Server-resolved tool definitions must survive the internal transport
and the worker projection into portable Agent definitions; adding tools only
before transport silently loses them in a separate-worker deployment. The tools
stay directly callable alongside the Bashkit shell.

A message's browser binding scopes the tool to the Agent mounted beside that
send. Reading returns the current unsaved instructions, branding, configuration,
starters, initial files, network settings, and sandbox policy as untrusted data.
These explicit reads become durable tool results in the conversation, like other
tool output. Dedicated credential/setup inputs, connection grants, and independent
integration resources are outside the adapter. Sandbox policy is readable but
edited through its dedicated form. The patch schema owns the supported fields;
unknown fields fail before any form value changes. Reads and resulting drafts are
bounded to 256 KiB; oversized forms fail explicitly rather than truncating values.

Patches require the binding and exact current draft revision. Manual edits,
refreshed server definitions, or write restrictions invalidate stale revisions.
A patch preserves omitted fields and enters the ordinary edit mode. It makes no
API save. The existing Save validates and persists the complete draft; Discard
resets it. A separate draft notice and field markers identify Chat's unsaved work.
Draft Undo restores the preceding form values, including earlier manual edits,
and refuses to overwrite manual edits made afterward. A manual edit between Chat
patches establishes a new Undo baseline.

The conversation must project a client-tool request into its waiting state; an
earlier session activation cannot mask that pause. Resuming model work returns
the UI to active. Otherwise the browser can receive the request without ever
executing it.

The browser executes only the latest runtime request correlated to the authored
message and a binding it owns. A different page, organization, or browser cannot
reuse the old adapter. Other browsers leave the request for its originating
browser; closing that browser relies on the normal client-tool timeout. Mixed
page/interactive client batches return errors without applying changes or
answering questions. Result delivery retries reuse cached results and never
reapply a patch. When a delivery settles, execution rechecks requests that arrived
while the HTTP reply was pending; failed deliveries still require explicit Retry. This is a typed runtime protocol, never assistant-prose parsing,
DOM scraping, or an extension of the separate WebMCP confirmation workflow.

The Platform Chat prompt routes adjacent-Agent requests through this adapter,
reads manager context before editing, and reports that the result is unsaved.
An unavailable or stale draft is not permission to fall back to a server write.
Without a browser adapter, the existing platform-command editing flow remains.

## Attributing and undoing Agent changes

Use [entity history](../execution/change-reasons-and-manager-context.md), which
already records the originating session and numbered snapshots. Do not infer
saves by matching tool-output prose, and do not use the administrator audit log.
While Chat is open, the Agent adapter polls history and refreshes the existing
entity query when a new revision appears. Only revisions attributed to this
permanent Chat receive gold markers. A different caller's revision establishes
a new baseline. Multiple Chat saves between polls belong to the same inverse.

The initial implementation marks definition **fields**: instructions, primary
configuration, and supported More rows and their settings sheets. A page toast
counts those fields. Dismissal clears presentation state and leaves saves intact.
Markers survive Back to app while the page remains mounted. A reload establishes
a fresh observation baseline; historical changes remain available in History.

Undo uses the existing restore command with a selected field list and expected
Agent revision. It acquires the Agent row lock before reading the current
revision; ordinary Agent updates acquire the same lock before reading the
definition. Validation, permissions, manager context, history, and the inverse
commit in one transaction. A stale revision fails with a conflict. A selected
field that the update API cannot restore also fails and rolls back completely;
it must never disappear from the UI as if the inverse succeeded. Secrets are
excluded. Undo all restores only the marked fields and preserves unrelated
configuration. Each inverse is a new history entry.

Draft values remain unsaved until the operator uses Save. If the saved Agent changes while a dirty draft is
open, keep the draft and block Save until Discard reloads the saved definition.
Dismissing Chat's toast does not remove this conflict. Otherwise an operator
could silently save an older prompt over the Chat's work.

## Integration decisions and next slices

- **Runtime revisions:** the exploration assumes old sessions are pinned. The
  current runtime records the starting revision for traces but resolves live
  Agent configuration between turns. Keep that runtime contract during this UI
  refactor; revision pinning requires an explicit runtime decision. Do not claim
  that an edit retroactively changes a running turn.
- **Granularity:** field undo is the initial safe inverse. Per-line prompt
  inverses and individual tag-chip inverses need a finer change representation;
  the prototype's per-line gold marking is not yet the shipped contract.
- **Navigation from Chat:** entity links already open in the adjacent page.
  Autonomous page navigation needs an explicit, bounded UI command protocol;
  never navigate by scraping arbitrary shell output or assistant prose.
- **Related resources:** MCP capabilities/configuration stored in the Agent
  definition follow its history. Credential values, independent connection
  grants, channel state and other child resources require their own attribution
  adapters and safe inverse policies; restoring an Agent must not restore a secret.
- **Other pages:** add typed context first, then history attribution and field
  markers, then inverse support and pane-scoped sheets. Reuse this shell for
  Harnesses, registries and operational surfaces; do not clone it per page.
- **Model choice:** Chat mode hides the model chip when a model resolves; retain
  the recovery picker when none is configured. It uses the existing managed
  session/model resolution instead of creating another model setting.

## Verification

Focused tests cover persistent shell/page state, organization reset, metadata
placement and retry preservation, attribution to this Chat, saves between polls,
partial undo, stale failure and dismissal. Server tests cover selected-field
restore, conflict rejection, and fields that cannot be restored. Draft tests cover
live unsaved reads, atomic patches, version and permission conflicts, preserving
manual edits through Undo, bounded values, and retry without duplicate execution.
Manual rollout
verification must still exercise a real model save, settings sheets, manual-draft
conflicts, light/dark modes, mobile fallback and reduced motion on a running stack.
