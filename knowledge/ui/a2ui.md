---
type: Specification
title: "A2UI Generative UI"
description: "Shared A2UI 1.0 component rendering, interaction and compatibility contracts for console chat and Work Chat."
tags: [everruns, ui, a2ui]
---
# A2UI Generative UI

## Contract

The opt-in `a2ui` capability teaches the agent to emit A2UI 1.0 operations in
fenced Markdown. Console chat and Work Chat use the **same** renderer package,
[`apps/ui/generative-ui`](../../apps/ui/generative-ui/), with CopilotKit's pinned
69-component standard catalog. The shared package lives inside the console's
Docker build context; Work Chat consumes it as a local dependency. Both image
builds copy that source before installing frozen dependencies.

AG-UI is the event transport, not a component catalog. Rendering fenced A2UI
needs only the existing text stream. No CopilotKit agent service, commercial
self-managed-agent API, or additional execution endpoint is used. The local
`/generative-ui/info` routes return empty renderer discovery metadata; actual
runs, authentication, messages and approvals stay on the existing Everruns APIs.

## Catalog and agent prompt

The agent's default prompt comes from the generated
[standard catalog guide](../../crates/core/src/builtins/a2ui/standard-catalog.md).
The [exporter](../../apps/ui/generative-ui/scripts/export-catalog.mjs) derives
signatures from the exact installed catalog and its Markdown guide. Its check
mode detects prompt/catalog drift. Component schemas, not a hand-maintained
parallel Rust list, own the new catalog. The older Rust catalog/prompt helpers
remain available for consumers supplying custom legacy prompts.

Browser dependency upgrades must refresh the generated guide, the vetted Zod v3
compatibility manifest, and the shared catalog fixtures. Both installations run
the existing authenticated security backport; its unknown-byte checks remain on.

## Surfaces and interaction

`createSurface`, `updateComponents`, `updateDataModel` and `deleteSurface` are
processed by the upstream validating surface manager. Bindings, browser
functions, input ownership and validation use that same engine. Multiple fences
in one reply can update one surface. Inline surfaces are scoped to the reply,
so a later answer does not silently rewrite earlier chat history. The shared
host also exposes named targets for embedding applications, but the two chat
hosts advertise inline surfaces only.

A2UI user events forward their visible message and resolved context through the
host's existing send function. They never execute generated backend commands.
Busy, archived and read-only views disable message actions. The console adapter
gets identity from session context, including Chats routes without a `sessionId`
URL parameter. Work Chat retains its consumer API, draft/attachment isolation,
idempotency and retry policy.

OpenUI uses the same package. Its actions forward visible messages and form
values through the same callback. No OpenUI Query/Mutation tool provider is
registered. Neither `callRendererFunction` nor `callAgentFunction` operations
are supported. Rendering is not authorization for an agent action.

## Compatibility, streaming and containment

Saved nested `{type, props, children}` trees still render through one shared
legacy adapter. This is an Everruns compatibility format, not a claim of full
A2UI v0.9 protocol conformance. Legacy forms submit values as chat messages.
Partial JSON recovery is shared, and completed standard operations are applied
as they arrive. Both hosts preserve fenced code examples instead of interpreting
widget markers inside them. Malformed output falls back to source or a visible
error; block, surface, operation and tree limits bound rendering work.

Host semantic colors drive light and dark component styling. Static HTML previews
remove navigation metadata and link destinations before applying the iframe
sandbox and CSP. Maps use the
installed MapLibre package and retain the upstream place-list fallback. Work
Chat's CSP permits HTTPS image/media sources, the standard map tile host, and
YouTube's privacy-enhanced player. HTML views are overridden with an opaque,
script-free iframe whose CSP also denies network requests, matching workspace
preview containment. Generated SVG and URLs retain the catalog's validation.

## Verification

- [Protocol tests](../../apps/ui/generative-ui/tests/protocol.test.ts): fence
  parsing, streaming, bounds, surface isolation and repeated updates.
- [Legacy renderer tests](../../apps/ui/src/__tests__/a2ui-renderer.test.tsx):
  form defaults/edits, URL containment and read-only behavior.
- [Console browser test](../../apps/ui/e2e/generative-ui.spec.ts) and
  [Work Chat browser tests](../../apps/chat-ui/e2e/chat.spec.ts): shared fixtures
  cover all 69 standard components; real browser interactions exercise forms,
  bindings, charts, tabs, updates and iframe containment.

See [OpenUI](openui.md), [Work Chat](../integrations/work-chat.md), and the
[public guide](../../docs/features/generative-ui.md).

The [public component gallery](../../docs/features/generative-ui-gallery.mdx)
uses the same shared renderer in isolated, lazy-loaded preview frames. Controls
work locally, and message actions display their payload without an agent or API
write. External media examples require activation before contacting their hosts.
Frames follow the documentation theme and can reset their local state. Static
captures remain available for release illustrations; the capture script is
[`capture-generative-ui-gallery.mjs`](../../apps/ui/scripts/capture-generative-ui-gallery.mjs).
