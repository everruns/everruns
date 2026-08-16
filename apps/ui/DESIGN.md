---
version: alpha
name: Slate
description: Everruns design system, sharp-cornered, grayscale-dominant, navy primary with a single gold accent.
colors:
  # Mirrors the light-mode CSS custom properties in src/app/design-system.css,
  # which is the runtime single source of truth. Values use hsl() to match those
  # variables exactly; dark-mode equivalents are documented in the Colors prose.
  background: "hsl(0 0% 98%)"
  foreground: "hsl(0 0% 10%)"
  card: "hsl(0 0% 100%)"
  card-foreground: "hsl(0 0% 10%)"
  popover: "hsl(0 0% 100%)"
  popover-foreground: "hsl(0 0% 10%)"
  primary: "hsl(220 62% 13%)"
  primary-foreground: "hsl(0 0% 98%)"
  secondary: "hsl(0 0% 96%)"
  secondary-foreground: "hsl(0 0% 10%)"
  muted: "hsl(0 0% 96%)"
  muted-foreground: "hsl(0 0% 45%)"
  accent: "hsl(43 60% 53%)"
  accent-foreground: "hsl(43 60% 30%)"
  destructive: "hsl(0 84% 60%)"
  destructive-foreground: "hsl(0 0% 98%)"
  border: "hsl(0 0% 88%)"
  input: "hsl(0 0% 85%)"
  ring: "hsl(43 60% 53%)"
typography:
  # Font families come from CSS variables: --font-sans (Geist Sans) and
  # --font-mono (Geist Mono). Body copy carries -0.01em global tracking.
  headline-lg:
    fontFamily: Geist Sans
    fontSize: 1.875rem
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: -0.02em
  headline-md:
    fontFamily: Geist Sans
    fontSize: 1.5rem
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: -0.015em
  title:
    fontFamily: Geist Sans
    fontSize: 1.125rem
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: -0.01em
  body-lg:
    fontFamily: Geist Sans
    fontSize: 1rem
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: -0.01em
  body-md:
    fontFamily: Geist Sans
    fontSize: 0.875rem
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: -0.01em
  label-md:
    fontFamily: Geist Sans
    fontSize: 0.75rem
    fontWeight: 500
    lineHeight: 1
    letterSpacing: 0em
  code:
    fontFamily: Geist Mono
    fontSize: 0.875rem
    fontWeight: 400
    lineHeight: 1.6
rounded:
  # Sharp corners are a defining brand trait: every radius is 0.
  none: 0px
  sm: 0px
  md: 0px
  lg: 0px
spacing:
  # 4px base scale (Tailwind defaults are used in practice); the branded dot
  # grid background repeats on a 24px cell.
  xs: 4px
  sm: 8px
  md: 16px
  lg: 24px
  xl: 32px
  2xl: 48px
  dot-grid: 24px
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
    rounded: "{rounded.none}"
    typography: "{typography.label-md}"
    padding: "0.5rem 1rem"
  button-primary-hover:
    backgroundColor: "hsl(220 62% 13% / 0.9)"
  button-secondary:
    backgroundColor: "{colors.secondary}"
    textColor: "{colors.secondary-foreground}"
    rounded: "{rounded.none}"
    typography: "{typography.label-md}"
    padding: "0.5rem 1rem"
  input:
    backgroundColor: "{colors.card}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.none}"
    padding: "0.5rem 0.75rem"
  card:
    backgroundColor: "{colors.card}"
    textColor: "{colors.card-foreground}"
    rounded: "{rounded.none}"
    padding: "1rem"
---

# Slate Design System

Slate is the Everruns design system. The normative token values live in the YAML
front matter above and in `src/app/design-system.css` (the runtime source of
truth); the prose below explains how to apply them.

## Reference and authority

Use the [Everruns design project](https://claude.ai/design/p/019e0a57-7d4b-70aa-a3c2-8c3d3746126e)
as a visual reference for Slate's page archetypes, density, and component anatomy. Exported design
project archives are snapshots for review; they are not copied into the application and do not
override repository sources.

The reviewed ZIP export is pinned by filename, SHA-256 checksum, and review date in
`design-reference.json`. Update that manifest whenever a newer export is adopted, then reconcile
intent here and verify both `/dev/design-reference` and `/dev/ui-components`.

When references disagree, use this order:

1. `src/app/design-system.css` for runtime tokens, utilities, and animation.
2. This file for visual intent, hierarchy, and composition rules.
3. Production components in `src/components/` for exact behavior and component APIs.
4. `/dev/ui-components` for an interactive rendering of those production components.
5. The external design project for visual comparison and page-level examples.

The external project should stay recognizably close to the product, but application code remains
canonical. Reconcile meaningful drift here rather than introducing parallel CSS or mock components.

## Overview

Slate feels like precision instrumentation: sober, dense, and engineered. The
product manages and monitors AI agents, so the UI favors legibility and calm
over decoration. The personality is **architectural**: sharp corners, a
grayscale foundation, and a single gold accent reserved for moments that matter.
When a rule is not spelled out, prefer the restrained, professional, slightly
utilitarian choice over the playful or ornamental one.

## Colors

The palette is grayscale-dominant, anchored by a deep navy primary and a single
gold accent. Descriptive brand names map to systematic tokens as follows:

- **Navy (`primary`, light `hsl(220 62% 13%)` ≈ `#0A1636`):** The brand color,
  used for primary buttons, key actions, and the loading-wave ramp. In dark mode
  `primary` inverts to near-white (`hsl(0 0% 95%)`) so actions stay prominent.
- **Gold (`accent` / `ring`, `hsl(43 60% 53%)` ≈ `#D4A43A`):** The sole accent,
  reserved for active states, focus rings, highlights, and the dark-mode dot
  grid. Use it sparingly, never as a large text surface.
- **Ink (`foreground`, `hsl(0 0% 10%)`):** Core text on light surfaces.
- **Surfaces (`background` `hsl(0 0% 98%)`, `card` white):** A near-white page
  with pure-white cards. Dark mode shifts to a navy-tinted charcoal
  (`background hsl(220 15% 8%)`, `card hsl(220 15% 10%)`).
- **Muted (`muted` / `muted-foreground`):** Quiet grays for secondary text,
  metadata, and disabled states.
- **Destructive (`hsl(0 84% 60%)`):** Errors and irreversible actions only.

Every token has a light and dark value in `design-system.css`; the tokens above
capture the light theme as the canonical reference.

## Typography

Two families carry the system: **Geist Sans** for everything structural and
**Geist Mono** for code, logs, and telemetry. Do not introduce handwritten or display fonts.
Experimental features use the Lucide flask marker in navigation rather than a page-title stamp.

- **Headlines (`headline-lg`, `headline-md`):** Geist Sans Semi-Bold with tight
  negative tracking for an engineered, condensed feel.
- **Body (`body-lg`, `body-md`):** Geist Sans Regular. Dense SaaS surfaces lean
  on `body-md` (14px); long-form reading uses `body-lg` (16px). Global letter
  spacing is `-0.01em`.
- **Labels (`label-md`):** Compact Geist Sans Medium for buttons, chips, and
  metadata.
- **Code (`code`):** Geist Mono for inline code and code blocks.

## Layout

Layouts use a 4px base spacing scale (`xs`–`2xl`) consistent with the Tailwind
defaults already in use. Content sits on a near-white page textured by a branded
**dot grid**: navy dots at 8% opacity in light mode, gold dots at 10% in dark
mode, repeating on a 24px cell (`spacing.dot-grid`). Group related items into
white cards with generous internal padding to separate them from the textured
background.

## Elevation & Depth

Slate is intentionally **flat**. Hierarchy comes from tonal layering and
borders rather than decorative elevation: a near-white background, pure-white
cards, and 1px borders (`colors.border`). Inputs and buttons may use `shadow-xs`;
popovers and dialogs may rise as far as `shadow-md`. Content cards do not cast
shadows. Focus and active emphasis is signaled with the gold `ring`. Subtle
motion (row-enter, tool-pulse, loading-wave, and a progress sheen) conveys state
changes in place.

## Shapes

The shape language is **uncompromisingly sharp**: every corner radius is `0px`
(`--radius` and all `--radius-*` tokens). Icons reinforce this with squared
line caps and mitered joins (`.icon-sharp`). The only round elements are semantic
status dots, circular Lucide glyphs, and the rings in the Everruns logo.

## Components

- **Buttons:** `button-primary` is navy with near-white label text and sharp
  corners; it uses 90% opacity on hover (`button-primary-hover`). `button-secondary` uses
  the light gray `secondary` surface for lower-emphasis actions. Both use the
  `label-md` token. Action hierarchy is semantic: `accent` creates a new entity,
  `default` commits changes, `outline` handles secondary actions, and destructive
  list actions use a red ghost icon rather than a filled button.
- **Inputs:** White surface, `input` border color, sharp corners, comfortable
  padding; focus draws the gold `ring`.
- **Cards:** White surface on the textured background, 1px border, no radius,
  no shadow. Card action groups keep the highest-priority action visible and
  collapse secondary or destructive actions into an ellipsis menu when the
  card is too narrow for the full action row.

### Composition invariants

- Top-level entity screens compose the five-zone primitives in
  `src/components/layout/page-layout.tsx`: breadcrumb, masthead, control strip,
  body columns, and footer. List, detail, and edit pages supply content without
  inventing a parallel shell.
- Domain cards compose shared primitives. For example, `AgentCard` composes
  `EntityCard`, which composes `Card`; previews must render `AgentCard` when they
  claim to show an agent card.
- Entity-specific styling belongs in the domain component, while navigation,
  identifiers, responsive action behavior, border treatment, and card anatomy
  belong in the shared primitive.
- Create actions use one `accent` button at the far right. Edit forms use one
  `default` commit button followed by an `outline` discard action. Read-only
  action clusters are otherwise `outline` or `ghost`.
- Headings, buttons, menu items, and badges use sentence case. API-facing names
  and identifiers remain verbatim and use Geist Mono when code-adjacent.

## Do's and Don'ts

- **Do** keep every corner sharp.
- **Do** reserve gold (`accent`) for active states, focus rings, and highlights;
  use navy (`primary`) for the single most important action per screen.
- **Don't** place dark text on a gold surface or use gold for body text, its
  contrast is too low for WCAG AA.
- **Don't** add decorative shadows to content cards or exceed `shadow-md` on overlays.
- **Do** edit `src/app/design-system.css` and this file together, they must
  stay in sync.
- **Don't** create lookalike domain examples from lower-level primitives; render
  the real domain component.
