# Brand Specification

## Abstract

Brand guidelines for the Everruns project ensuring visual and verbal consistency across all properties (landing page, documentation, UI, GitHub).

## Requirements

### Name & Tagline

1. **Name**: Everruns
2. **Tagline**: "Durable AI. Unstoppable agents."
3. **Meaning**: AI agents that **ever run** — continuous, uninterrupted, eternal execution

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

- Follows shadcn/ui conventions
- Dark mode support
- Gold for success/active states
- Navy for primary actions

### External Resources

| Resource | URL |
|----------|-----|
| Geist Font | https://vercel.com/font |
| Astro (Landing/Docs) | https://astro.build |
| Cloudflare Pages | https://pages.cloudflare.com |
