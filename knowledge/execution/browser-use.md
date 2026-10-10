---
type: Specification
title: "Browser Use"
description: "Provider-neutral browser use: agents read pages as an accessibility tree with element refs and act by ref, by coordinate, and across tabs."
tags:
  - everruns
  - execution
  - capabilities
  - safety
---

# Browser Use

Status: neutral tool and native Claude adapter implemented, behind the
`browserless_browser_use` flag.
Capability `browser_use`, contract in
[`crates/contracts/src/runtime/browser_use.rs`](../../crates/contracts/src/runtime/browser_use.rs),
Browserless backend in
[`crates/integrations/src/browserless/browser_use/`](../../crates/integrations/src/browserless/browser_use/mod.rs).
Tracked as EVE-1133 (Claude browser toolset parity).

## Why

[Computer use](computer-use.md) sees a page only as pixels, so every step costs
a screenshot and a coordinate guess. Anthropic's `browser_toolset_20260801`
reads the page structure instead: an accessibility tree whose elements carry
refs the model acts on, plus form filling and tabs. Everruns should offer that
to every model, not only to Claude, and run it on the browser a session already
has.

## What

- **One `browser` tool** whose action vocabulary, targets
  (`{type: ref}` or `{type: coordinate}`), result texts and limits follow the
  toolset reference, so a later native Claude adapter maps members one to one.
  The four members Anthropic ships off (`javascript_exec`, `file_upload`,
  `read_console`, `read_network`) are refused by name.
- **Every success carries a browser state report**: all tabs, exactly one
  active, and `tab_opened` changes since the last report. This is the
  `browser_state` block's shape.
- **Same browser as computer use and the Browserless tools.** Pointer and
  keyboard actions reuse `CdpDisplay`, so a ref click is the element's center
  clicked exactly as a `computer` click.

## Decisions

- **Refs live on the Everruns side.** A ref is a Chrome backend DOM node id in
  session storage (`browser_use.refs.<tab>`), keyed by the main frame loader
  id. The page cannot see or move it; a navigation or a removed node yields the
  reference's stale-ref error rather than an action somewhere else
  (TM-TOOL-060).
- **Tree from the accessibility API, visibility from one layout snapshot.**
  `Accessibility.getFullAXTree` gives roles and names as assistive technology
  sees them; one `DOMSnapshot.captureSnapshot` gives every layout box. Two calls
  per read regardless of page size.
- **Tool scripts run in an isolated world** so page overrides of built-ins do
  not change reads or writes.
- **`find` is word scoring, not a model call.** Cheap and deterministic; a
  query naming what an element says or does finds it.
- **Tabs are page targets of the guarded browser context**, with Chrome's
  target id as `tab_id`. Each call reconnects, so the active tab, known tabs and
  cursor are kept under the reserved `browser_use.` storage prefix.

## Native adapter

The capability contributes the `everruns/browser_use` driver option
([`crates/contracts/src/native_computer.rs`](../../crates/contracts/src/native_computer.rs))
unless `native_tools` is off. The Anthropic driver
([`crates/drivers/drivers/src/anthropic/browser_toolset.rs`](../../crates/drivers/drivers/src/anthropic/browser_toolset.rs))
swaps the `browser` function tool for the toolset entry on the models the
reference lists, turns member calls into `browser` calls, and on replay builds
each member result from the tool's JSON: text, images, and one
`browser_state` block. Execution, approval and budget never change.

- **Batches per toolset.** A response's browser member calls share a batch
  named after its first browser member call; after a failure the rest answer
  with the browser toolset's own skip text. The failed batch lives in its own
  storage key, so a failed computer call does not skip browser calls.
- **No `configs`.** The four members the API ships off are the four the tool
  refuses, so the entry sends the default member set.
- **Provider-side parallel tool use is off** while either toolset is active:
  member actions depend on the page the previous one left.

## Phases

| Phase | Scope | State |
|---|---|---|
| 1 | Neutral `browser` tool on Browserless, capability `browser_use` | Done, experimental |
| 2 | Native Anthropic `browser_toolset_20260801` adapter with `browser_state` blocks, batches stop at the first failure | Done, experimental |

## Rejected options

- **Refs written into the DOM as attributes.** Simple to resolve, but any page
  script can read and rewrite them.
- **Selectors as targets.** Fragile across renders, and the toolset contract
  uses refs.
