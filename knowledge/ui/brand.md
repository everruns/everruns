---
type: Specification
title: "Brand Specification"
description: "Brand identity, colors, typography."
tags:
  - everruns
  - ui
---
# Brand Specification

## Abstract

Brand guidelines for the Everruns project ensuring visual and verbal consistency across all properties (landing page, documentation, UI, GitHub).

## Requirements

### Name & Tagline

1. **Name**: Everruns
2. **Tagline**: "Durable Agentic Harness. Unstoppable agents."
3. **Meaning**: A harness engine where AI agents **ever run**: continuous, uninterrupted, eternal execution

### Logo

Three interlocking rings (Borromean rings pattern) representing **Durability × Scalability × Reliability**.

#### Centering invariant (mathematically correct)

**Every logo asset is centered on the rings' centroid**: the 3-fold rotational
symmetry center, which is also the gold convergence point, **not** the bounding
box. The centroid is placed at the exact center of its frame so the mark stays
balanced in circular and square crops (favicon, avatar, app icon, GitHub org
avatar). Bounding-box ("optical") centering is **wrong** here: because two rings
sit low and one sits high, it leaves the centroid ~3.5% below center.

For three equal circles whose centers form an equilateral triangle, the centroid
is the mean of the three centers. The horizontal centroid is already on the
center line; only a vertical shift is needed.

#### Canonical geometry (512 viewBox, centroid at 256, 256)

| Ring | center (cx, cy) | r | stroke |
|------|-----------------|---|--------|
| Top | `256, 183.64` | 120 | 18 |
| Bottom-left | `193.33, 292.18` | 120 | 18 |
| Bottom-right | `318.67, 292.18` | 120 | 18 |

Round caps/joins, `fill: none`. Equilateral triangle of centers: side `d = 125.34`,
circumradius `R = 72.36`. (Pre-centering the centroid was `274.09`; it was shifted
up `18.09` to reach `256`. Exact gradient/stop values live in the source SVGs.)

- **Color** (`logo.svg`): three `userSpaceOnUse` linear gradients, base navy →
  gold (`#D4A43A`), converging at the canvas center.
- **Mono** (`logo-mono.svg`): single stroke `#0A0A0A`.

#### Variants (each centroid-centered in its own canvas)

| Variant | Canvas | Notes |
|---------|--------|-------|
| Standard color / mono | 512, transparent | canonical geometry above |
| Railway avatar (`template-icon.svg`) | 512, navy `#0A1636` rounded square (rx 112) | rings `r=116` stroke `28`, white `#F8FAFC` + gold; centers `256,192` / `188,288` / `324,288` |
| OG banner (`og-image.svg`, `generate-og-image.mjs`) | 1200×630 social card | logo placed via group transform; rings centroid-centered internally, transform compensates so banner layout is unchanged |

#### Source files (edit these)

| File(s) | Variant |
|---------|---------|
| `logo.svg`, `apps/ui/public/logo.svg`, `apps/docs/src/assets/logo.svg`, `apps/docs/public/favicon.svg`, `apps/ui/src/app/icon.svg` | color (identical) |
| `logo-mono.svg`, `plugins/everruns/assets/everruns-small.svg`, `plugins/everruns-dev/assets/everruns-small.svg` | mono (identical) |
| `infra/railway/template-icon.svg` | Railway avatar |
| `apps/docs/src/assets/og-image.svg`, `apps/docs/scripts/generate-og-image.mjs` (`LOGO_SMALL`) | OG banner |

#### Derived rasters (regenerate after editing source)

| Raster | Source | Regenerate |
|--------|--------|-----------|
| `logo-1024.png` | `logo.svg` | render `logo.svg` at 1024×1024 (sharp) |
| `apps/docs/public/og-image.png` | `og-image.svg` | `cd apps/docs && node scripts/generate-og-image.mjs` (also runs in `prebuild`) |
| `apps/ui/src/app/favicon.ico` | `icon.svg` | render at 16/32/48 → ICO |
| GitHub org avatar | `logo-1024.png` | upload manually in org settings |

#### Rule for new logo derivations

Any new logo instance must be **centroid-centered**: place the centroid at the
center of its frame (or, in a composed layout, at the center of the intended
area). When in doubt, copy `logo.svg` / `logo-mono.svg` and only adjust the
surrounding canvas, never re-derive ring coordinates by bounding box.

### Color Palette

#### Primary Colors (Use sparingly, for accents only)

| Name | Hex | Usage |
|------|-----|-------|
| Navy | `#0A1636` | Links, primary actions, dark backgrounds |
| Gold | `#D4A43A` | Highlights, success states, accents |

#### Grayscale (Primary UI colors)

| Name | Hex | Usage |
|------|-----|-------|
| Obsidian | `#0A0A0A` | Primary text, headers |
| Charcoal | `#1A1A1A` | Dark backgrounds, footer |
| Slate | `#404040` | Secondary text, borders |
| Silver | `#A0A0A0` | Muted text, captions |
| Smoke | `#F5F5F5` | Section backgrounds, cards |
| White | `#FFFFFF` | Primary background |

### Typography

**Font Family**: Geist Sans / Geist Mono (free from Vercel)

```css
:root {
  --font-sans: 'Geist', system-ui, sans-serif;
  --font-mono: 'Geist Mono', monospace;
}
```

**Type Scale**:

| Element | Weight | Size |
|---------|--------|------|
| H1 | 600 | 2.5rem |
| H2 | 600 | 2rem |
| H3 | 500 | 1.5rem |
| Body | 400 | 1rem |
| Code | 400 | 0.875rem |

**Line Height**: 1.6 for body text

### Voice & Tone

1. **Confident**, not arrogant
2. **Technical**, but accessible
3. **Direct**: say it simply
4. **Calm**: we handle chaos so you don't have to

#### Writing mechanics

Prose should read as written by a person, not a model. Applies to public docs
(docs.everruns.com), `knowledge/`, `README`s, and UI copy.

1. **No em-dashes (`—`).** Use a comma, colon, semicolon, parentheses, or two
   sentences. Pick the punctuation that fits the clause. (Hyphens in compound
   words and en-dashes in numeric ranges are fine.)
2. **Avoid AI-tell coinages and filler.** Do not use "seam", "load-bearing",
   "seamless(ly)", "delve", "tapestry", "realm" (as a metaphor), "testament",
   "underscore(s)" (as a metaphor), "boasts", "elevate", "supercharge",
   "cutting-edge", "game-changer", "meticulous(ly)", "pivotal", "unlock" (as a
   metaphor), "empower", or "it's worth noting". Say the plain thing: name the
   interface or extension point, a feature is enabled not "unlocked". Keep real
   terms of art where they carry precise meaning, and reword the metaphor
   everywhere else.

### Design Principles

1. **Simple and clean**: no fancy colors, gradients only in logo
2. **Grayscale dominant**: content-first, minimal distraction
3. **Generous whitespace**: let content breathe
4. **Mobile-first**: responsive, works on all devices
5. **Fast**: minimal JavaScript, optimized images

### UI Design System (Slate)

The "Slate" design system defines the visual language for the Everruns UI application.

#### Sources of truth and DESIGN.md

The Slate design system has two co-located, machine-readable sources of truth in
the UI app:

- `apps/ui/src/app/design-system.css`, the **runtime** source of truth: CSS
  custom properties, theme tokens, utilities, and animations consumed at build
  time. Downstream apps import this file rather than duplicating it.
- `apps/ui/DESIGN.md`, the **agent- and human-readable** source of truth in the
  [DESIGN.md format](https://github.com/google-labs-code/design.md) (YAML token
  front matter + design-rationale prose). Its tokens mirror the light-mode values
  in `design-system.css`.

These two files MUST stay in sync: when changing a token in `design-system.css`,
update `DESIGN.md` in the same change (and vice versa). Validate `DESIGN.md`
structurally and against WCAG contrast with `pnpm run design:lint` (in
`apps/ui`), which runs `@google/design.md lint`. This spec captures intent only;
do not duplicate the full token tables here, read them from those two files.

#### Corners & Radius

**Sharp corners (0px)** throughout for a clean, developer-focused aesthetic.

```css
--radius: 0px;
--radius-sm: 0px;
--radius-md: 0px;
--radius-lg: 0px;
```

#### Active State Pattern

Navigation and interactive elements use **left/right border accents** instead of background fills:

```css
/* Active navigation item - left border */
.nav-active {
  background: hsl(43 60% 53% / 0.1);
  color: hsl(43 60% 30%);
  border-left: 2px solid hsl(43 60% 53%);
}

/* User message - right border */
.user-message {
  background: hsl(43 60% 53% / 0.1);
  border-right: 2px solid hsl(43 60% 53%);
}
```

#### Color Tokens

| Token | Light Mode | Dark Mode | Usage |
|-------|------------|-----------|-------|
| `--primary` | Navy (#0A1636) | Light gray | Primary buttons, actions |
| `--accent` | Gold (#D4A43A) | Gold | Active states, highlights, focus rings |
| `--background` | Off-white (98%) | Dark blue-gray (8%) | Page background |
| `--muted` | Light gray (96%) | Dark gray (15%) | Hover states, disabled |
| `--border` | Gray (88%) | Dark gray (18%) | Borders, dividers |

#### Hover States

Use `bg-muted` for hover states instead of accent colors to keep UI calm:

```css
/* Correct */
hover:bg-muted hover:text-foreground

/* Avoid */
hover:bg-accent hover:text-accent-foreground
```

### Branded Background

Subtle dot grid pattern applied to all surfaces (app, docs). Provides texture without distraction.

```css
/* Light mode - Navy dots */
background-image: radial-gradient(
  circle at center,
  hsl(220 62% 13% / 0.08) 1px,
  transparent 1px
);
background-size: 24px 24px;

/* Dark mode - Gold dots */
background-image: radial-gradient(
  circle at center,
  hsl(43 60% 53% / 0.1) 1px,
  transparent 1px
);
background-size: 24px 24px;
```

| Property | Light | Dark |
|----------|-------|------|
| Color | Navy | Gold |
| Opacity | 8% | 10% |
| Dot size | 1px | 1px |
| Grid spacing | 24px | 24px |

### Application Guidelines

#### Landing Page (everruns.com)

- Hero: White background, Obsidian text
- Features: Smoke background
- Footer: Charcoal background, Silver text
- Accents: Gold for highlights, Navy for links

#### Documentation (docs.everruns.com)

- Light theme: White/Smoke backgrounds, Navy links
- Dark theme: Navy background, Gold accents
- Code blocks: Subtle borders, monospace font

#### UI Application

- Follows shadcn/ui conventions with Slate design system
- Dark mode support
- Sharp corners (0px radius) throughout
- Gold for active states (nav items, focus rings, running indicators)
- Navy for primary actions (buttons)
- Left/right border accents for active navigation and messages
- Muted gray for hover states (not accent colors)

#### Transcript Surfaces

Chat transcript styling stays **inline and single-surface**:

- Tool activity rows (`read_file`, `write_file`, `list_files`, search, bash, client tools) should render as lightweight transcript lines, not nested cards inside message cards.
- Todo/progress state should render as one inline block with a thin progress indicator and rows beneath it, not a card containing more boxed rows.
- Avoid **double-wrapped boxes** in the transcript. If a message already establishes a surface, tool and todo content inside that area should not introduce another bordered card unless the content truly needs an isolated canvas (for example, a full image preview or modal).
- Prefer status glyphs, border accents, spacing, and muted text over stacked borders/background panels.

#### Primary Action Buttons

Primary "create new" actions (New Session, New Agent, etc.) use consistent styling:

```tsx
<Button variant="accent">
  <Plus className="h-4 w-4 mr-2" />
  New [Entity]
</Button>
```

- **Variant**: `accent` (gold background)
- **Icon**: Plus icon, `h-4 w-4` size, `mr-2` margin
- **Label**: "New [Entity]" format (e.g., "New Session", "New Agent")

### Figma Design System

The canonical Figma file for the Everruns design system lives in the **Everruns** team project:

| Asset | URL |
|-------|-----|
| Component Library & Agents Page | https://www.figma.com/design/ib5pQ5VGUcbsqiWPDt6Ds7 |

**File structure:**

- **Page 1, Component Library**: Color palette (13 tokens, light/dark), typography scale (7 levels), button variants (7 × 3 sizes), badge variants (5), inputs (text, search with ⌘K), card component, capability chips, navigation items (active/inactive), and Agent Card component.
- **Page 2, Agents Page**: Full desktop (1440px) recreation of the `/agents` route with sidebar, populated agent cards, and example agents section. Uses real Lucide SVG icons imported from `lucide-react`.

**Design tokens** are defined as Figma variables with Light and Dark modes in the "Slate Design System" collection, matching `apps/ui/src/app/globals.css`.

**Agent Card component** (`Component Library` page) mirrors `src/components/agents/agent-card.tsx` with exact Tailwind spacing: `py-6` (24px), `gap-6` (24px), `px-6` (24px), `text-lg` (18px) title, `text-sm` (14px) description, `text-xs` (12px) badges/chips/tags.

### External Resources

| Resource | URL |
|----------|-----|
| Geist Font | https://vercel.com/font |
| Astro (Landing/Docs) | https://astro.build |
| Cloudflare Pages | https://pages.cloudflare.com |
