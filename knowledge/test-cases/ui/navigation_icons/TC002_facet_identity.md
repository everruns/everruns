---
type: Test Case
title: "TC002: Facet domain identity and vector exports"
description: "Verify the original Intent Agent mark and custom domain outlines across navigation, entity fallbacks, themes, responsive layouts, and SVG downloads."
tags:
  - everruns
  - test-case
  - ui
  - navigation-icons
---
# TC002: Facet domain identity and vector exports

## Preconditions

- Full DB-backed local development stack with authentication disabled.
- A seeded Agent without an uploaded avatar and a session.

## Test Data

Use the current gallery at `/dev/icons` and seeded Agent/session data.

## Steps

1. Open `/agents`. Compare the sidebar Agent glyph with the masthead and avatar fallback:
   all use the original solid Intent planes. Open an Agent and a session; inspect Agent
   references in the transcript/header. Uploaded avatars, if present, remain images.
2. Open command search via the Search control. Query Agents, Models, and an Agent ID, then
   a session ID. Confirm page/entity/ID results use the relevant domain glyph; a session ID
   must not display an Agent glyph. Existing visibility and navigation behavior still work.
3. Inspect Settings: Account and My agent experience are distinct; Organization, Team,
   Providers, Health, Features, and access tokens use domain outlines. MCP and Slack retain
   their official marks.
4. Open `/dev/icons`. Inspect every master at 16, 20, 24, and 32 pixels. Check distinct
   silhouettes, clear negative space, square terminals, and unclipped paths. Download Agent
   and Skills SVGs and confirm their paths and paint match the displayed master.
5. Switch to dark mode and repeat the sidebar, Settings, gallery, and search checks. Icons
   inherit foreground color and remain legible in selected and muted states.
6. Use a narrow viewport, open the navigation drawer, and repeat domain checks. Confirm
   alignment, labels, close/reopen behavior, and successful navigation.

## Expected Result

- One original Agent identity across fallback surfaces; supporting domains use the same
  outline on every relevant surface, with official integration marks retained.
- Icons remain clear at actual menu size in both themes and the responsive drawer.
- SVG downloads are editable vectors matching React; there are no external resources or
  raster artwork. Icon controls remain named and decorative icons do not duplicate labels.
