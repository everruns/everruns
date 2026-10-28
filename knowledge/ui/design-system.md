---
type: Design
title: Slate Design System Pages
description: Why Slate lives as public design system pages built from the app's tokens and icons, and the contract that keeps them current.
tags:
  - everruns
  - ui
  - brand
  - design-system
---

# Slate Design System Pages

## Abstract

Slate, the visual language of the Everruns UI, is published as a section of the docs:
[everruns.com/docs/design-system/](https://everruns.com/docs/design-system/), with its source in
[`docs/design-system/`](../../docs/design-system/) and the example components in
[`apps/docs/src/components/design-system/`](../../apps/docs/src/components/design-system/).
The pages cover foundations, components, page layouts, icons and brand, and UI writing. Every
redesign starts from them, and people and agents consult them before changing the UI.

## Why

Slate was first captured in an external design tool export (Claude Design). That tool is being
retired, the export was a snapshot that drifted from main within weeks (navigation groups,
Facet icons, chat turns, focus ring), and nothing in the repository could check it. The repo
already held the runtime tokens ([`design-system.css`](../../apps/ui/src/app/design-system.css))
and composition rules ([`DESIGN.md`](../../apps/ui/DESIGN.md)), but no browsable place showed
how the parts fit together or gave a redesign a starting point.

## Decisions

- **Public docs section, not an internal folder.** The product is open source, so contributors
  get the same reference; the pages ship with the docs build, get the per-page Markdown copies
  and their own `llms.txt` set, and agents can read either the published pages or the MDX
  source.
- **Tokens and icons are read, never copied.** `apps/docs/scripts/copy-design-tokens.mjs`
  extracts the light and dark custom properties plus the dot grid, `.icon-sharp`, and motion
  classes from `design-system.css` at build time, scoped to `.ds-example`. The Facet catalog and
  every domain glyph render from
  [`facet-icon-data.ts`](../../apps/ui/src/components/icons/facet-icon-data.ts). A token or
  icon change in the app reaches the site in the same build.
- **Example markup is a hand reproduction.** The examples are static HTML with `ds-` classes
  (`apps/docs/src/styles/design-system-examples.css`) that mirror the Tailwind output of the
  production components. Rendering the real React components would pull the Next.js app into
  the Astro docs build; a reproduction is cheaper and is also the editable starting point a
  redesign needs. The cost is that layouts and components can drift; the contract below covers
  it.
- **The export is not committed.** Its JSX lookalikes, copied CSS, and preview cards would
  have been a second, already stale source. The original archive is kept outside the repository
  for history only. `apps/ui/design-reference.json`, which pinned it, is removed.
- **No design skill.** The pages and this concept are the reference; the workflow below is
  short enough to live here.

## Contract

1. A redesign starts on the design system pages: mock the change with their classes and
   examples, get it approved, then build it from the production components.
2. A pull request that changes a documented UI pattern (component anatomy, page layout,
   navigation groups, writing rule) updates the matching page under `docs/design-system/` and,
   when needed, `design-system-examples.css`, in the same change.
3. Token, utility, motion, and Facet changes need no page edit; check that the rendered pages
   still read correctly.
4. Authority order when sources disagree: `design-system.css`, then `DESIGN.md`, then the
   production components, then the pages. Product decisions for individual screens stay in
   their own concepts ([agent page](agent-page.md), [chat experience](chat-experience.md),
   [information architecture](information-architecture.md)); the pages link to them.

## Known gaps

- The docs site keeps its own subset of Slate tokens in `apps/docs/src/styles/slate.css` for its
  chrome. It differs slightly from the app (muted foreground, focus ring). The design system
  examples are not affected because they use the app tokens.
- Layout examples render inside the docs content column (about 720px), narrower than the app's
  content area, so wide layouts wrap earlier than in the product.


## Reusable product patterns

Resource pickers compose the shared Base UI combobox; feature wrappers own filtering,
permissions, pagination and missing-value repair. Single selection, multiple bindings, tags,
ranges and local view choices have distinct shared controls. This prevents each feature from
reimplementing popup, keyboard and feedback behavior. Public component guidance owns the
selection rules; the development gallery renders the actual implementations.

Standard pages use the five-zone page-layout primitives, including query loading and failure
branches. The older page-shell API is retired. Settings keeps one navigation model with a
compact destination selector on narrow screens. Chat and session workspaces remain intentional
layout variants. Domain drafts and save policies remain owned by their features.

Confirmation requests are scoped to route and organization and live inside the page Chat context.
This cancels stale requests on navigation and preserves the existing manual-dialog guard.
