# Brand Specification

## Abstract

Brand guidelines for the Everruns project ensuring visual and verbal consistency across all properties (landing page, documentation, UI, GitHub).

## Requirements

### Name & Tagline

1. **Name**: Everruns
2. **Tagline**: "Durable Agentic Harness. Unstoppable agents."
3. **Meaning**: A harness engine where AI agents **ever run** — continuous, uninterrupted, eternal execution

### Logo

Three interlocking rings (Borromean rings pattern) representing **Durability × Scalability × Reliability**.

| File | Description | Location |
|------|-------------|----------|
| `logo.svg` | Color version with navy-to-gold gradients | `apps/ui/public/logo.svg` |
| `logo-mono.svg` | Black & white version | Future |
| `favicon.svg` | Use logo or logo-mono | Future |

### Color Palette

#### Primary Colors (Use sparingly — for accents only)

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
3. **Direct** — say it simply
4. **Calm** — we handle chaos so you don't have to

### Design Principles

1. **Simple and clean** — no fancy colors, gradients only in logo
2. **Grayscale dominant** — content-first, minimal distraction
3. **Generous whitespace** — let content breathe
4. **Mobile-first** — responsive, works on all devices
5. **Fast** — minimal JavaScript, optimized images

### UI Design System (Slate)

The "Slate" design system defines the visual language for the Everruns UI application.

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

### External Resources

| Resource | URL |
|----------|-----|
| Geist Font | https://vercel.com/font |
| Astro (Landing/Docs) | https://astro.build |
| Cloudflare Pages | https://pages.cloudflare.com |
