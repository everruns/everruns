---
type: Decision
title: "Agent Avatars"
description: "Why agent avatars are rendered once at upload into square and circular presets behind public immutable URLs, and how they reach the Agent Card and Slack."
tags:
  - everruns
  - ui
  - agents
  - slack
  - a2a
---

# Agent Avatars

## Abstract

An agent has at most one avatar. Upload crops it to a square once and renders every preset (square
and circle at 32, 64, 128, 256, 512 px, plus the square source at up to 1024 px) as PNG. The
presets are served from `/v1/avatars/{avatar_id}/{variant}`, unauthenticated and cached as
immutable. Sources: [`avatar.rs`](../../crates/server/src/domains/agents/avatar.rs),
[`agent_avatars.rs`](../../crates/server/src/api/agent_avatars.rs),
[`avatar_slack.rs`](../../crates/server/src/domains/agents/avatar_slack.rs).

## Decisions

- **Render at upload, never on read.** Serving is one keyed lookup. A request-time resizer would
  need its own cache and a decode-bomb budget on the hot path.
- **A new upload gets a new id.** URLs never change meaning, so `Cache-Control: immutable` is safe
  and a CDN or Slack's image proxy can keep them forever. The replaced avatar is deleted.
- **Public reads.** Slack, A2A clients and email cannot send our credentials. The id is a fresh
  UUID only learned from the agent or the surfaces it already publishes to.
- **Square is canonical; the circle is pre-masked.** Surfaces that cannot clip (Slack, A2A
  clients) get the same circle the UI shows. Everything is PNG: Slack rejects WebP, and the circle
  needs alpha.
- **Crop in the browser, center-crop on the server.** The Branding sheet takes a dropped or chosen
  file, frames it with `react-easy-crop`, and uploads only the square (PNG, up to 1024 px). The
  server center-crops whatever arrives, so API uploads need no client step.
- **Variants live in PostgreSQL.** They are tens of KB each, so `bytea` beats the object store's
  extra pointer rows and GC.
- **Slack gets it as the app icon.** Each Slack endpoint is its own Slack app, and a manifest cannot
  carry an icon. With one-click provisioning the 512 px preset goes through `apps.icon.set` when
  the app is created and on every avatar change, best effort (Slack allows about one call a
  minute). Removing an avatar leaves the last icon, since Slack has no reset. Manually created
  apps keep Slack's default icon.
- **Agent Cards.** The A2A card carries the 256 px square as `iconUrl` on the card's own origin.
  The MCP agent card inlines the 64 px square as a data URI because its CSP blocks fetches.

- **Curated sources share the upload pipeline.** Branding offers a searchable catalog of 25
  avatars in five visual families. Stable IDs, names, descriptions and keywords are owned by
  [`avatar_presets.rs`](../../crates/server/src/domains/agents/avatar_presets.rs). Selection accepts
  only an exact catalog ID, retains that identity with the stored avatar, and persists the same
  square/circle variants and Slack updates as upload. No client-supplied source URL is fetched.
- **Search is presentation only.** Each whitespace-separated term can match any metadata field,
  case-insensitively. Tiles and selected previews show artwork only; metadata remains available to
  search and assistive technology. Roles never change the agent's behavior or configuration. Current
  selection survives reload; custom images remain uploadable, replaceable and removable.

## Open
- Avatars generated from the display name were considered and left out for now.
