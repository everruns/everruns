---
type: Decision
title: "Page-aware Chat mode"
description: "How the Adoption-gated split shell connects a Platform Chat instance to the current page, with shared context across app pages and typed draft editing."
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

The authenticated app shares one context and registration contract. Read-only
views, editable forms and host-composed pages reuse the same transcript, composer
and navigation model. Adoption does not make immediate page actions model-callable.

## Rollout and ownership

`chat_mode` is a separate **Adoption** flag, off until the organization opts in.
It does not require or enroll `chat_threads`. Deployment availability alone must
never enable it, including during organization loading. Turning it off restores
the existing shell. The source catalog is
[`feature_flags.rs`](../../crates/server/src/records/feature_flags.rs).

The app layout owns one long-lived
[`ChatModeShell`](../../apps/ui/src/components/chat-mode/chat-mode-shell.tsx)
and one [`ChatWorkspace`](../../apps/ui/src/components/chat/chat-workspace.tsx).
Chat and every subchat are full conversation destinations: `/chats` opens the
permanent conversation; `/chats/{id}` replaces it with the selected instance.
Neither opens another conversation beside itself. History is a list destination.
Command/Ctrl-J on a Chat destination focuses its existing composer.

A **subchat** is a fresh instance of the managed Platform Chat Agent with its own
session, transcript and workspace files. Creating one does not fork, seed or copy
another transcript. The shared/private memory contract still applies; a new
conversation is not a new identity or a separate organization. New subchat and
recent subchats remain discoverable regardless of either Adoption flag. An empty
draft creates its session only on first send; a failed first send reuses that
session. Ordinary subchats keep normal pin/archive behavior.

Command/Ctrl-J on an app page opens the selected personal Chat instance beside
that page, defaulting to permanent Chat. The shell retains one mounted conversation
and its composer when moving between full Chat and the page workspace. Back to
app hides the conversation without tearing down its stream or composer. An
organization switch destroys the shell's state. The mounted conversation settles
already-owned page tool requests even after the split closes: the same original
binding may finish, while navigation returns an unavailable-page result instead
of leaving the turn waiting or touching a different form. Full Chat never claims
a new page binding. Delegated child conversations
are not selected as page editors. Page context is enabled only beside non-Chat
pages, and the conversation list does not register filters as adjacent-page state.

The Subchats panel lists independent conversations. Its optional **Work** tab
shows delegation and background tasks belonging to the active Chat instance, or
the validated Platform Chat parent of a delegated child. It never hosts a second
conversation or a second composer. Existing task links remain usable without
automatically enabling new delegation.

The core **Settings → Chat** page uses the same guarded draft, saved review and
Undo flow for its independent Chat model. That setting applies to permanent Chat
and fresh subchats; it does not replace agent/session defaults. A new subchat's
model preview waits for organization settings and follows the same default
selection order as the server. See [model resolution](../foundations/models.md).

### Proposed flag schema (not migrated)

The existing keys remain compatible and retain their current organization
rollouts. Their labels now describe the behavior: **Chat workspace** (`chat_mode`)
and **Chat background work** (`chat_threads`). The proposed key cleanup is:

| Surface | Proposed key | Availability |
| --- | --- | --- |
| Chat, fresh subchats, history, pin/archive | None | Core, always available |
| Page-side Chat, page context, draft/review tools | `chat_workspace` | Organization Adoption opt-in; replaces `chat_mode` |
| Delegation capability and Work panel | `chat_background_work` | Organization Adoption opt-in; replaces `chat_threads` |
| External public-facing Chat | `public_chat` | Existing independent rollout; unchanged |

There is no master Chat gate and neither Adoption feature depends on the other.
Voice remains its cross-cutting capability flag. A future migration should resolve
an explicitly configured new key first, then fall back to its old key when no new
value exists. Do not OR aliases: an explicit false must win. Preserve enrollment,
sessions and history; never enroll an organization as part of the rename.

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
not grow an independent page catalog. Embedding hosts may supply their existing
composed navigation sections, so stricter host visibility and loading rules
remain effective in the rail. Shared role, feature and policy checks still apply. Agent settings drawers mount within the
page pane with a page-only backdrop, leaving Chat accessible.

Every rail destination has a short explanation in the shared navigation model.
Hover or keyboard focus reveals its name and purpose in a tooltip to the left of
the rail. The always-labeled Pages control opens the same permitted destinations
as a grouped menu with names and descriptions, including on touch screens.
Escape dismisses an overlay and the menu returns focus to its trigger. Closing
Chat removes rail overlays without remounting the page or conversation. Discovery
does not prefetch destination content or render hidden, disabled or restricted pages.

## Page context is data

[`ChatModeContext`](../../apps/ui/src/components/chat-mode/chat-mode-context.tsx)
is an optional page adapter. Every page within the adopted authenticated shell
receives a browser binding, route and resource identity. Existing page-title hooks
supply readable labels. Explicit forms register their own identity and fields;
registration cleanup removes only its own adapter, so nested pages and dialogs do
not clear a parent's context.

An explicit read can return bounded summaries from active, successful query-cache
entries for the current organization and resource. A fixed projection excludes
credentials, configuration objects, conversation bodies and file contents; it is
not a cache dump or a background fetch. Operational/global queries without a known
tenant scope are omitted. Summary limits and truncation are reported. A right-pane
registry reads explicit React filters/tabs and mounted entity references; it never
scrapes the DOM or observes inputs in Chat, the sidebar or credential controls.

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

## Shared forms and read-only views

[`usePageFormDraft`](../../apps/ui/src/components/chat-mode/use-page-form-draft.tsx)
connects explicit, strict field schemas to existing React setters. The controller
has no API client or Save callback. Text-only integrations project an independent
field-name allowlist, so passing additional local state cannot expose a credential.
The returned patch schema describes the fields actually supported on that form;
unknown fields, invalid values, read-only fields and stale revisions fail before
any setter. Closing and reopening a form rotates its binding even when values
are identical; captured adapters from the preceding lifecycle remain unavailable.
The channel type chooser exposes no draft until a type is selected. Returning to
the chooser expires that editor, including when the same type is selected again.
Patches preserve omitted values. A notice names changed fields and
offers guarded draft Undo, preserving later manual edits. Saved-baseline conflicts
keep dirty work and block the integration's Save action until it is discarded.

Beyond Agents, integrations cover Harness creation/editing, Virtual Users,
Observer definitions, Eval creation, Model ID/display-name dialogs, declarative Capability metadata/skills/files,
Knowledge collection/source/account forms, Memory and legacy Knowledge-index
metadata dialogs, Skill creation, Sandbox-template metadata and non-credential
Channel text. Each exposes its supported subset; MCP headers/environment values,
API keys, channel auth, sandbox environment values and grants remain manual.
Existing scorer variants unsupported by the current Observer editor remain readable
with their scorer collection read-only; metadata edits preserve them.
Credential-bearing URL fields disable their adapter and incoming URL patches are
validated. No redacted credential is ever written back as a form value.

Tracing provides bounded destination status rows and a routing draft with normal
Save/Discard and history. Organization defaults retain their fixed routing mode.
Loading or failed policy reads mask underlying editors, including cached-data
failures. Receiver credentials, content capture and delivery actions remain manual.
Interface-language drafts use the registered locale catalog and affect this device
only; execution locale and resource history remain separate.

Providers have independent details, tracing and advanced Save forms. Their shared
page binding returns named forms and permits one named form per patch. A collection
uses the page title for its context chip, rather than the last registered form. Child
validation and the aggregate size check run before mutation. Opening/closing or
replacing a form rotates the collection binding. A dialog may take priority over
the underlying page form, then unmount cleanly back to that page.

After Adoption, Organization, Model defaults and Web search defaults stage both
mouse and Chat edits locally behind explicit Save/Discard controls. With Adoption
off they retain autosave. Organization Profile fields, invitation and account names,
provider connection metadata, model selection, MCP metadata, schedule editors,
eval cases, report-builder settings, Playground setup and queue metadata use the
same draft contract. Channel variants expose typed public configuration while
preserving nested authentication settings. Grants and approval policy remain manual.

Clean secondary forms follow refreshed saved values; dirty forms retain their
work and block Save on a changed baseline. Closing/reopening starts a new binding.
Credential and approval-only popups register a read-only context above underlying
forms, so typing beside a protected popup cannot edit the page underneath it.
Credential values, auth methods, queued/scheduled task payloads and generated tokens stay outside
these projections. Setup, financial actions, imports and uploads retain their
normal manual controls.

Operational views project the status rows actually displayed, with explicit load,
error and truncation state. Global durable summaries require the platform policy;
a tenant administrator role alone is insufficient. Monitoring reflects its visible
live, paused or replay state. These bounded scalar projections supplement the
existing scoped query summaries and never include arbitrary payloads or errors.
Settings navigation and viewport-based grids stack to the available page pane.
Account identity/configuration keep stable cache keys across organization switches,
so a later account refresh does not remount the page; see
[Authentication](../security/authentication.md). Organization label/role refreshes
update the active membership without switching its identity or resetting page caches.

Other operational pages, credential setup, approvals and financial actions
provide read-only context and retain their normal page controls. A missing
writable form is not permission to save the adjacent page through server commands.
Separately requested saved-resource operations retain the normal Platform Chat
command flow; a failed form patch must never be reinterpreted as permission for
a direct server save.

### Shared saved review

Personal Settings follow the split navigation: Account is the global private account name; Profile is the organization virtual-user profile. Console Profile saves resolve `/me` to the proved self binding before the normal revision guard and journal. Members can review their own profile without management permission; self-service cannot restore lifecycle. Runtime self-service writes share the resource lock, and unrecorded changes invalidate saved Undo. Connections, MCP servers and AI clients retain their existing protected projections.

Every supported existing-resource editor uses the same Adoption-only review
controller as Agents. A local draft shows Before/After values with field and
whole-draft Undo. Normal Save stays a user action. Before Save, the browser checks
the resource revision captured when editing began; the server checks it again
under the same resource lock ordinary writers and inverse Undo use. The response
identifies the exact committed revision, so a later writer cannot enter this
browser's review accidentally. Independent settings forms sharing an organization
also share that organization's conflict boundary.

Saved reviews survive page navigation within the organization. They show
Before/After, per-field Undo, Undo all and History. Undo requires the reviewed
revision, the resource's current edit authority and no dirty local draft. A newer
save refuses the inverse without overwriting it. A failed review read after a
successful Save is reported as saved; reloading history rebuilds the review
without resubmitting the mutation. Only the active page and mounted forms poll;
retained review groups are bounded. Organization changes dispose their receipts
and reject late callbacks. Reload keeps server history, while temporary review
markers start afresh.

The exact resource mappings live in
[`page-resource-history.ts`](../../apps/ui/src/components/chat-mode/page-resource-history.ts).
Model selection applies reviewed boolean changes in one transaction and returns
per-model revisions; Undo all is also atomic. It compares the picker-open values,
so a background catalog refresh cannot silently replace that baseline. Account
names and Sandbox Template metadata have inverses that preserve current credentials
and execution specifications. Channel configuration inverses use an explicit public
projection merged into current configuration; tokens, sign-in and approval policy
are never replayed. Provider diagnostics preserve current headers. Account profile history
is private to the account owner, independent of the active tenant, and is included
in account export and erasure.

Creation and irreversible actions retain draft review followed by their existing
submit controls. Undo does not delete a newly created resource, revoke an issued
token, unsend an invitation, replay a payment or dequeue a task. Protected settings
and arbitrary execution payloads remain manual. An inverse whose old values no
longer pass normal validation is refused with the review retained. Guarded schedule
Undo requires the durable PostgreSQL store to share the command transaction;
external stores and remote MCP subscription effects require direct review.

Dialogs and drawers originating in the right page portal into its stable container.
Chat, sidebar and global overlays retain their normal portal and modal behavior. While Chat is open they are
non-modal and outside-pointer dismissal is disabled: focusing Chat keeps the form
mounted. Explicit Close/Escape still work. The common two-column page body responds
to the page pane's width. Feature-off pages retain their normal controls and shell.
Dialogs scroll within the available page height, keeping Save and Close reachable
when schedule previews, validation messages or draft diffs increase their content.

## Coverage of new interfaces

Authenticated pages have an explicit coverage inventory in
[`chat-page-coverage.json`](../../apps/ui/src/test-utils/chat-page-coverage.json).
Its regression gate fails on added or moved pages until their editor, view,
conversation, manual-action or redirect contract is reviewed. It tests the common
fallback binding and refusal behavior; a catalog entry is not a rendered-page or
live-model acceptance result. Field fixtures independently enumerate the shared
schemas and exercise each field, omitted-value preservation, read-only access,
revoked authority and stale baselines. Rendered editor and browser tests
complement these common contracts.

The tabbed Knowledge interface shares those contracts. Collection settings follow
clean refreshes and preserve dirty drafts on concurrent updates. Collection,
account, source and search views expose bounded displayed metadata. Search excerpts,
source configuration, credential bindings and raw errors stay outside those
projections. The shared file browser follows only expanded visible ancestors and
bounds its projection before walking a large catalog; file bodies and file-write
actions retain their manual controls. Its layout uses the page pane's width.

Agent and Harness display translations use a bounded locale-keyed draft schema;
translations cannot alter execution instructions or stable resource identity.
They follow the resource's ordinary Save, saved review and guarded Undo. Interface
language is a separate device preference: Adoption stages it until Save, while
feature-off behavior still switches immediately. It has no server entity history.

Manager notes acquire their own high-priority sheet binding. They are readable
only when the authorized sheet is open, writable only after Edit, and retain the
notes endpoint's own expected revision on Save. Draft diffs and Undo stay local;
notes writes remain context-history entries rather than resource-snapshot inverses.
Protected history, sandbox and file overlays mask underlying writable forms and
restore their bindings on close. Installation health exposes status and check
identity only after platform-policy authorization; diagnostic payloads remain out.

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

## Attributing saved changes

[Entity history](../execution/change-reasons-and-manager-context.md) records the
originating session and numbered snapshots. The shared review controller follows
only changes attributed to the active Chat instance or a correlated browser Save. It
never infers a mutation from tool-output prose or the administrator audit log.
A foreign writer ends the uninterrupted Chat sequence, including writes between
polls. Undo can reach only the Chat sequence after that writer.

Agents retain inline gold field markers for instructions and supported settings;
all pages use the shared saved-review panel described above. Dismissal clears
presentation state and leaves saved configuration and history intact. A reload
starts a fresh observation baseline while History remains available.

Selected-field Undo runs through the resource's normal update validation,
permissions and manager context. Unsupported old values refuse the inverse and
keep its receipt; the UI must not treat a failed inverse as a successful Undo.
Replacement-style update commands receive current values for unselected settings
so a field inverse cannot reset other saved configuration through input defaults.
Secrets and protected authority remain current. A dirty form keeps its values
when the saved definition changes and blocks Save until Discard rebases it.
Dismissing review markers does not remove that conflict.

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
  grants and child resources use their own authorization and inverse policies;
  restoring an Agent must not restore an independent resource or a secret.
- **Resource history:** shared context and form drafts do not imply saved-change
  attribution or inverse Undo. Add those only with that resource's authorization
  and revision-guarded history contract; do not clone the shell per page.
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
