---
type: Specification
title: "Computer Use"
description: "Provider-neutral computer use: agents see a display through screenshots and act on it with pointer and keyboard actions."
tags:
  - everruns
  - execution
  - capabilities
  - safety
---

# Computer Use

Status: phase 1 implemented, behind experimental mode. Capability
`computer_use`, contract in
[`crates/core/src/computer_use.rs`](../../crates/core/src/computer_use.rs), first
backend in
[`integrations/browserless/src/computer.rs`](../../integrations/browserless/src/computer.rs).
Tracked as EVE-1119.

## Why

Computer use was the headline harness capability of OpenAI DevDay 2026 (the
Agents API, GPT-6.1 Sol, GPT-6 Astra) and Anthropic ships it as a GA toolset.
Everruns sandboxes run commands and edit files, and Browserless drives pages
through selectors, but until now no agent could look at a screen and click.
Selector-driven tools fail on canvas apps, shadow DOM, and anything the model
cannot name. A screenshot plus coordinates works on every page.

The differentiator is neutrality: one capability that works with OpenAI,
Anthropic, and Gemini models, on displays Everruns controls, instead of each
vendor's hosted variant.

## What

### Contract

- **Action vocabulary.** `ComputerAction` mirrors Anthropic's computer-use
  member names and argument names (`left_click` with `coordinate: [x, y]`,
  `scroll_direction`, `scroll_amount`, `key` with `ctrl+a` combos), plus
  `navigate` for browser displays. It is the richest published set, and it maps
  one to one onto OpenAI's `computer_call` actions, so a native adapter
  translates only the envelope.
- **One tool.** The `computer` tool is a single function tool with an `action`
  discriminator. Any model that accepts tool results with images can use it,
  with no provider support at all. Native adapters (below) swap the definition
  and keep the execution path.
- **Backends.** `ComputerBackend::acquire` hands the tool a `ComputerSession`
  for one call: `perform`, `screenshot`, `release`. The backend owns display
  lifetime (the Browserless backend reuses the session's persistent browser).
- **Coordinates are screenshot pixels.** Backends render at exactly the
  configured size with a device scale factor of 1. Actions are validated
  against the display before they run.
- **Every action returns a screenshot** by default, so the model acts on the
  frame it just produced. `screenshot_after_action: false` returns text for
  actions and images only for `screenshot`.

### Budget

Screenshots are image tokens. Config caps the display (default 1280x800, max
1920x1200) and the number of actions per session (`max_actions_per_session`,
default 300, screenshots included). The counter lives in session storage, under
a key reserved from the model-facing `kv_store` tool so the model cannot reset
its own cap, and is charged only after an action validates, before a display is
acquired.

### Safety

The model sees everything on the screen, and on-screen text is the main
prompt-injection path. Layers, weakest to strongest:

1. The capability prompt marks screen contents as untrusted data and tells the
   model to stop and ask before typing credentials, purchasing, sending, or
   confirming irreversible actions. Pair it with
   [soft approval](soft-approval.md) for a recorded yes.
2. The tool declares `open_world`, so the interactive
   [`tool_approval`](capabilities.md) gate asks before every call at the
   `normal` level.
3. `action_requires_approval` names the actions that commit input (typing,
   Enter, navigation) for hosts that want a per-action gate instead of a
   per-tool one. Clicks are not gated: gating every click makes the capability
   unusable, and a dangerous click is covered by layers 1 and 2.

Hosted sessions can enforce the per-call gate ([tool approval](tool-approval.md),
TM-TOOL-008), but it is opt-in per agent and nothing yet requires it for
computer use or gates per action, which is why the capability is still
experimental. Egress follows the session's network access list;
see TM-TOOL-048 to TM-TOOL-050 in the [threat model](../security/threat-model.md).

### Browserless backend

The display is one Chromium page on the session's persistent Browserless
browser, driven over CDP. Text goes in with `Input.insertText`; keys carry
key, code, and virtual key code so default actions fire (Enter submits). The
CDP layer is independent of Browserless and is tested against a local headless
Chromium, filling and submitting a form end to end.

## Phases

| Phase | Scope | State |
|---|---|---|
| 1 | Contract, `computer` function tool, Browserless browser backend | Done |
| 2 | Native adapters: OpenAI `computer` tool (`computer_call` / `computer_call_output`) and Anthropic `computer_toolset_20260801`, selected by model profile | Next, builds on EVE-1115's hosted tool support |
| 3 | Desktop backend on a sandbox image (Xvfb plus a screenshot bridge) for non-browser apps; per-action hard approval on top of the hosted [tool approval](tool-approval.md) gate | Planned |

## Rejected options

- **A tool per action.** Seventeen tools crowd the tool list and make native
  adapters map names both ways. One tool with a discriminator is how both
  vendors model it on the wire anyway.
- **OpenAI hosted computer use only.** The hosted tool still needs a client-side
  executor, and building it OpenAI-only would leave Anthropic and Gemini agents
  without the capability.
