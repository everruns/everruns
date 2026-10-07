---
type: Decision
title: "Harness Page"
description: "Why the harness page reads and edits a harness in the agent workspace, with capabilities as the page."
tags:
  - everruns
  - ui
  - harnesses
---

# Harness Page

## Abstract

The harness page uses the agent workspace, with a different center. Capabilities are the wide
main pane, because a harness is the capability set a session starts from. The system prompt is
optional, so it is a More row. A narrow config column holds parent harness, model, and tags.
Viewing and editing share that layout; edit mode turns the same panes writable instead of routing
to a separate form. Source:
`apps/ui/src/app/(main)/harnesses/[harnessId]/page.tsx`, with its parts in
[`apps/ui/src/components/harnesses/`](../../apps/ui/src/components/harnesses/). The agent layout
this follows is [Agent Page](agent-page.md).

## Why

The harness detail page gave the prompt, capabilities, and configuration equal cards, and editing
meant a second page that arranged the same fields differently. Choosing capabilities, the usual
reason to open a harness, meant scrolling past a prompt the harness may not even have. Built-in
harnesses and custom ones also did not make read-only versus editable obvious.

## Decisions

- **Capabilities are the page.** The wide pane lists local capabilities, and edit mode turns it
  into the selector. Parent harness, default model, and tags stay in the config column. The
  system prompt, branding, starter files, network access, and usage are **More** rows. An empty
  prompt says the harness contributes none.
- **Page-level edit mode.** Edit, change several things, then one **Save changes** or
  **Discard**. Untouched layers are omitted from the update. `system_prompt` is sent only when it
  changed, including an empty string that clears a previous prompt. The old `/harnesses/{id}/edit`
  route redirects to `?mode=edit`. Built-in harnesses stay read-only, including when that link is
  opened.
- **One tab row:** Harness, Preview, Integrate, Stats. Integrate stays the headless calling guide.
  Archive and delete live in the header overflow menu. There is no test-chat action: a session
  still needs an agent.
