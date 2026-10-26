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

Status: phases 1 to 4 implemented; the browser backend is behind experimental mode. Capability
`computer_use`, contract in
[`crates/contracts/src/runtime/computer_use.rs`](../../crates/contracts/src/runtime/computer_use.rs), first
backend in
[`crates/integrations/src/browserless/computer.rs`](../../crates/integrations/src/browserless/computer.rs),
Sandbox desktop (capability `sandbox_desktop`, a Sandbox Template option) in
[`crates/capabilities/src/capabilities/sandbox_desktop.rs`](../../crates/capabilities/src/capabilities/sandbox_desktop.rs),
with desktop drivers in
[`crates/integrations/src/e2b/computer.rs`](../../crates/integrations/src/e2b/computer.rs) and
[`crates/integrations/src/daytona/computer.rs`](../../crates/integrations/src/daytona/computer.rs).
Tracked as EVE-1119 (phase 1) and EVE-1133 (phase 2).

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
- **Batches.** A call may carry `actions: [...]` (at most 16) instead of one
  `action`; this is how OpenAI's multi-action `computer_call` arrives. A batch
  is validated and budgeted whole, runs in order, stops at its first failed
  action, and always answers with one frame. `zoom` images come before that
  frame; a lone `zoom` answers with the region and a lone `cursor_position`
  with text, since neither changes the screen.

### Native adapters

The capability contributes a provider-neutral driver option,
`everruns/computer_use` with the display size
([`crates/contracts/src/native_computer.rs`](../../crates/contracts/src/native_computer.rs)),
unless `native_tools: false`. A driver swaps the `computer` function tool for
its native tool only when the option is set, the call offers `computer`, and
the model has the native tool; every other driver ignores the option and the
function tool keeps working. Execution never changes: a native call becomes a
call of the `computer` tool, so the budget, the approval gate and the backend
are the same on every path.

- **OpenAI** (Responses API with hosted tools, GPT-5.4 and later, GPT-6):
  `{"type": "computer"}`. A `computer_call` becomes a batched `computer` call;
  the result goes back as `computer_call_output` with a `computer_screenshot`.
  It is client-executed, so it is never a hosted-call event and never priced
  as one. Provider safety checks travel in the arguments, gate the call, and
  are acknowledged on replay only when the call ran. Wire details the GA docs
  do not pin down are isolated in
  [`crates/contracts/src/openai_computer.rs`](../../crates/contracts/src/openai_computer.rs).
- **Anthropic** (`computer_toolset_20260801`, the models in
  `anthropic_has_computer_toolset`): member calls (`left_click`, `type`, ...)
  carry `toolset_name: "computer"` and become `computer` calls with the member
  as `action`; replay reverses it and tags every result with `toolset_name`.
  All seventeen members are neutral actions. Claude sends a batch as several
  member calls in one response and expects the batch to stop at its first
  failure, so each converted call carries `native_batch` (the first member
  call's id); after a failure the tool answers the rest of that batch with the
  reference's "Not executed" text without touching the display. See
  [`crates/drivers/drivers/src/anthropic/computer_toolset.rs`](../../crates/drivers/drivers/src/anthropic/computer_toolset.rs).

Model support is a model-id rule next to each adapter rather than a model
profile flag: the native tools landed on a handful of current models, and the
option is safe to send everywhere.

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
   `normal` level when the agent enables it.
3. **Soft approval only: no per-call hard gate** (EVE-1133 decision,
   aligned with EVE-1140, which found hosted sessions have no hard gate at
   all). Layers 1 and 2 above carry approval: the prompt's stop-and-ask rule
   plus the `open_world` declaration, with [soft approval](soft-approval.md)
   for a recorded yes. A former durable per-call gate (TM-TOOL-008) was
   removed: gating every committing action made the capability unusable, and
   clicks always ran freely anyway.

Egress follows the session's network access list; see TM-TOOL-048 to
TM-TOOL-050 in the [threat model](../security/threat-model.md). The session UI
shows each result's screenshot as a thumbnail on the tool row, so a reviewer
sees what the agent acted on.

### Browserless backend

The display is one Chromium page on the session's persistent Browserless
browser, driven over CDP. Text goes in with `Input.insertText`; keys carry
key, code, and virtual key code so default actions fire (Enter submits). The
CDP layer is independent of Browserless and is tested against a local headless
Chromium, filling and submitting a form end to end.

### Desktop drivers

Both drivers run on the Session Sandbox itself (see Sandbox desktop below).
They began as standalone, flagged capabilities (`computer_use_desktop`,
`computer_use_daytona`) that each created a second, desktop-only sandbox; that
split the agent's shell and screen across two machines and let the desktop
escape the template's network rule, so both were removed in phase 3c.

- **E2B.** E2B's `desktop` template carries Xvfb, xdotool and an xfce session.
  The driver starts Xvfb at exactly the configured size, acts through
  `xdotool` argv via envd's process API (typed text is one argument after
  `--`, no shell), and reads frames back as PNG checked against the display
  size. The two fixed shell scripts (display start, screenshot) take only
  numbers and constant paths.
- **Daytona.** Daytona's default image ships Xvfb, xfce and x11vnc, sized at
  create time by `VNC_RESOLUTION`. Each action is one JSON request to the
  toolbox Computer Use API (`/computeruse/mouse/*`, `/keyboard/*`,
  `/screenshot`), so no shell sees model text; keys the daemon cannot name are
  refused, not emulated.
- **No `navigate`.** The display is not a browser page; the model opens a
  browser on the desktop.

### Sandbox desktop

Capability `sandbox_desktop`, added by the server, never chosen directly. A
Daytona or E2B Sandbox Template with `target.options.desktop` (`true` or the
`ComputerUseConfig` settings) makes the provider create the Session's primary
Sandbox with a desktop, and template resolution adds `sandbox_desktop` next to
`session_sandbox` with the same config
([`crates/capabilities/src/capabilities/sandbox_desktop.rs`](../../crates/capabilities/src/capabilities/sandbox_desktop.rs)).
Its `computer` tool asks the provider's `SessionSandboxProvider::open_desktop`
for a session on that Sandbox, through the Daytona or E2B session types above.

- **Why an option, not a flagged capability.** The standalone backends open a
  second, tool-owned Sandbox: other files, other lifetime, and only the
  session access list for egress. The desktop option puts the display on the
  machine the shell and file tools already use, under the template's network
  rule, with no extra Sandbox to clean up. It is opt-in per template, so it
  needs no feature flag.
- **One display.** It shares the `computer` exclusive group; resolution drops
  any other computer-use capability from the Agent's list instead of failing.
- **Display up front.** The provider starts the display as soon as the
  Sandbox runs and sets `DISPLAY` in its environment, not on the first
  `computer` action: agents open a browser from the shell before their first
  screenshot, and a browser started with no X server exits.
- **Provider images.** E2B defaults the Sandbox to its `desktop` template.
  Daytona uses its default image, which ships the desktop, so a desktop
  template rejects `size` and `snapshot`; the display size goes in through
  `VNC_RESOLUTION` at create.

## Phases

| Phase | Scope | State |
|---|---|---|
| 1 | Contract, `computer` function tool, Browserless browser backend | Done |
| 2 | Native adapters (OpenAI `computer`, Anthropic `computer_toolset_20260801`), batched calls, soft approval only with no per-call hard gate, screenshot thumbnails in the session UI | Done (EVE-1133) |
| 3 | Desktop backend on an E2B `desktop` sandbox (Xvfb, xdotool, PNG frames), capability `computer_use_desktop` | Done (EVE-1133), replaced by 3c |
| 3b | Desktop backend on a Daytona sandbox through its Computer Use API, capability `computer_use_daytona` | Done, replaced by 3c |
| 3c | Desktop as a Sandbox Template option on Daytona and E2B, capability `sandbox_desktop` | Done |
| 4 | Full Anthropic toolset: `zoom`, `cursor_position`, `left_mouse_down`/`up`, `hold_key`, modified drags, native batches stop at the first failure | Done |

Open before the capability leaves experimental mode: verify the native OpenAI
path against the live API (the GA reference leaves some action fields
unconfirmed).

## Rejected options

- **A tool per action.** Seventeen tools crowd the tool list and make native
  adapters map names both ways. One tool with a discriminator is how both
  vendors model it on the wire anyway.
- **OpenAI hosted computer use only.** The hosted tool still needs a client-side
  executor, and building it OpenAI-only would leave Anthropic and Gemini agents
  without the capability.
