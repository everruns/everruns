---
type: Decision
title: "Agent Page"
description: "Why the agent page reads and edits an agent in one layout with the system prompt as the page, and where every setting lives."
tags:
  - everruns
  - ui
  - agents
---

# Agent Page

## Abstract

The agent page is a workspace: the system prompt is the wide main pane and a narrow config column
holds everything else. Viewing and editing share that layout; edit mode turns the same panes
writable instead of routing to a separate form. Source:
`apps/ui/src/app/(main)/agents/[agentId]/page.tsx`, with its parts in
[`apps/ui/src/components/agents/`](../../apps/ui/src/components/agents/).

## Why

The old edit page gave every setting the same weight: identity, branding, behavior, starter files,
network access, the danger zone, capabilities, checks, and health check were bordered cards of
similar size over two columns. Changing the prompt or model, the most common task, meant scrolling
past branding and a large file browser. The detail and edit pages also arranged the same data
differently, so moving from reading to changing an agent meant finding your place again.

Two users come here: someone tuning an existing agent's prompt and model over and over, and
someone finishing a new agent. Both mostly touch a small set of fields; the rest is set once.

## Decisions

- **Hierarchy through placement, not decoration.** Primary settings (harness, capabilities in
  precedence order, default model, tags) are always visible. The capability list is a compact
  ordered set of chips, so a long set does not stretch the column; the editor stays in the column
  while the page is in edit mode, with tighter rows. Secondary settings are one **More** row each
  that shows its current value ("Inherited", "2 files") and opens a side sheet, so the whole
  configuration reads without opening anything. Side sheets, not accordions, because Branding and
  Files are large editors. Service account is a More row. Its sheet saves immediately, like MCP
  servers and Credentials, and stays editable on a built-in agent. The Files row, and the Files
  card on Preview, appear only when the effective agent has a filesystem: a capability that
  contributes one (including through a dependency or the harness) or starter files already set.
  Without a filesystem, the empty Files section stays hidden. Adding a filesystem capability
  reveals the row. Source: `apps/ui/src/components/agents/agent-files-visibility.ts`.
- **Page-level edit mode.** Edit, change several things, then one **Save changes** or
  **Discard**. A prompt edit and the capability change that goes with it land in one update.
  Changing any config control in view mode enters edit mode with that change pending, so nothing
  saves on its own. The old `/agents/{id}/edit` route redirects to `?mode=edit`.
- **Editable versus read-only is visible at a glance.** Editable values wear bordered controls;
  read-only facts, and every value on an archived agent, are plain muted text.
- **One tab row:** Agent, Preview, Integrations, Stats, Sessions. The selected tab is part of
  the address (`?tab=`), so a refresh or a shared link reopens it; the Agent tab omits the
  parameter and `/agents/{id}` stays the default. MCP servers and Credentials are configuration,
  so they are More rows; their sheets keep saving immediately as their own resources. Triggers
  stay under Integrations (EVE-1009). History, Manager notes and archive/delete live in the header
  [Entity Actions Menu](entity-actions-menu.md), replacing the danger-zone card. Old
  `?tab=mcp` and `?tab=credentials` links open the matching sheet; `?tab=versions` opens History.
- **The Sessions tab is the agent's full history.** It includes archived sessions because the
  tab badge counts them, and marks those cards as Archived. The ordinary Sessions and Chats
  lists still hide archived sessions by default. Cards show the source/channel, creation time,
  and latest conversational-message time; a session edit or tool event is not a new message.
- **Checks sit next to what they check.** In edit mode prompt findings render under the prompt
  editor; the behavioral health check is a More row.
- **Button tiers.** Gold is only **Test in Playground**. It opens Playground setup with the Agent
  selected; no session or Environment starts until the operator confirms the setup. Personal
  Chats remain bound to the managed Platform Chat. In edit mode navy Save changes replaces
  the action, with Discard beside it, and the header states that changes apply to new sessions
  only.

## Page-aware Chat rollout

Organizations enrolled in [Chat mode](chat-mode.md) can keep this page beside
Platform Chat. Chat reads and patches the page's live form, including unsaved
manual edits. Changes remain in the existing draft until Save, with separate
draft markers and guarded local Undo. Agent definition saves attributed to that
conversation refresh the page and receive persisted-change markers with guarded
undo. A concurrent saved change blocks a dirty draft's Save until the operator discards and
reviews the latest definition. Marker dismissal never bypasses that conflict.

- **Localization is a draft section.** A More row opens locale-keyed display-name and
  description overrides. Add or remove any supported language tag, then use the
  page-level Save changes or Discard. Built-in resources keep their read-only policy.
  Display text falls back per field; stable addresses and execution configuration
  remain independent.
