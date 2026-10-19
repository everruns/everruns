---
type: Design
title: Facet Iconography
description: The intent, visual grammar, and acceptance bar for Everruns' custom domain icons.
tags:
  - everruns
  - ui
  - brand
  - iconography
---

# Facet Iconography

Facet gives Everruns' domain entities their own visual identity within [Slate](brand.md).
The Agent is the central entity: its approved **Intent** mark consists of three separated,
ascending planes. It suggests direction and agency without a face, robot, eye, or mascot.
Keep the original proportions and negative space. The thinner Intent alternatives were
rejected; its solid silhouette anchors the quieter supporting family.

## Visual grammar

Supporting icons are restrained geometric outlines. Straight segments, small chamfers,
square terminals, mitered corners, and deliberate negative space should feel like a person
constructed and optically adjusted vector paths in Illustrator. Use curves only when they
clarify the subject. The family must suit Slate's sharp edges and dense information layouts.

The canonical masters use a 24-unit artboard and 1.75-unit outline weight. Judge the result
at the actual 16-pixel navigation size first; a beautiful enlarged drawing is insufficient.
Keep edges clear of the artboard, separate adjacent strokes, and remove detail that merges
at small sizes. The solid Agent is the intentional weight exception. Do not globally thicken
outline icons to make them compete with it.

Use recognizable domain metaphors with distinctive silhouettes: a framed environment for
a harness, a chip for models, a book for skills, modular units for capabilities, and a plug
for plugins. Related domains can share a construction motif, but must remain distinguishable:
sandbox fleet versus template, account versus virtual user, conversation versus session.
The exhaustive catalog and exact geometry belong to the
[vector masters](../../apps/ui/src/components/icons/facet-icon-data.ts).

## Product contract

- A domain has one glyph across navigation, Settings, search, page identity, and entity fallback.
  Navigation ownership remains in the [shared models](../../apps/ui/src/lib/navigation.ts),
  [Settings model](../../apps/ui/src/lib/settings-navigation.ts), and
  [registry model](../../apps/ui/src/lib/registry-navigation.ts).
- Glyphs inherit their surface's foreground color. Selected navigation uses the existing
  active state; it does not introduce a new icon accent palette. Both themes ship.
- Uploaded Agent avatars and deliberately chosen harness/capability icons retain their identity.
  Intent replaces the generic Agent fallback, not the user's image or a named integration logo.
- Official third-party marks (including MCP and Slack) remain official. Conventional action,
  direction, status, and file-type glyphs remain outside the domain family.
- SVG paths are static, reviewed source. No remote art, embedded images, fonts, filters,
  externally referenced resources, or database-provided markup is required to render them.
- Icons next to text are decorative. Icon-only controls need an accessible name on the control.
  The React components forward refs and ordinary SVG props.

## Reproduction and acceptance

[React components](../../apps/ui/src/components/icons/facet-icons.tsx), the development-only
[gallery](../../apps/ui/src/app/dev/icons/page.tsx), and standalone SVG exports all consume
one master set. SVG is the exchange format for Illustrator and other vector tools; raster
screenshots and generation boards are reference material, not runtime assets.

The [Facet extension workflow](../../.agents/skills/facet-icons/SKILL.md) owns the procedure
and an optional image-generation brief for ideation. A bitmap suggestion must be redrawn as
simple paths and reviewed in context before adoption. Never autotrace a raster into a noisy
menu icon or revise the Agent merely to match a newly generated family.

Acceptance requires legibility at menu size, distinctive neighboring silhouettes, matching
identity across consumers, theme and responsive checks, and parity between React and SVG
exports. The [manual cases](../test-cases/ui/navigation_icons/index.md) cover these surfaces.
