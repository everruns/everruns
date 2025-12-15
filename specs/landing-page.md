# Everruns Landing Page Specification

This specification defines the landing page for everruns.com. It is designed to be implemented in a **separate repository** by an agent with no access to the main Everruns codebase.

---

## Overview

### Project Summary

**Everruns** is a durable AI agent execution platform. It runs long-running LLM agents reliably — if the service restarts, agents resume from where they left off. Built on Temporal for durable execution.

**Core Value Proposition:**
- While others focus on making agents smarter, Everruns makes them **unkillable**
- Every step is persisted; agents survive crashes, restarts, and infrastructure failures
- Real-time streaming with AG-UI protocol compatibility

---

## Technical Requirements

### Static Site Generator

**Recommended: Astro**

Rationale:
- First-class Markdown/MDX support with content collections
- Excellent Cloudflare Pages integration (official adapter)
- Ships zero JavaScript by default (fast, simple)
- Easy to add interactive components later if needed
- Great for content-focused sites that will grow to multiple pages
- Future-proof for API docs integration (supports Starlight docs theme)

Alternative options (if Astro doesn't fit):
- **Hugo** — faster builds, but less flexible for future interactivity
- **11ty** — simpler, but less modern tooling

### Hosting

**Platform:** Cloudflare Pages

Configuration:
```
Build command: npm run build
Build output directory: dist
Node.js version: 20
```

### Content Structure

```
src/
├── content/
│   ├── pages/           # Markdown pages
│   │   ├── index.md     # Home page content
│   │   ├── about.md     # About page (future)
│   │   └── contact.md   # Contact page (future)
│   └── config.ts        # Content collection config
├── layouts/
│   └── BaseLayout.astro # Main layout
├── components/
│   ├── Header.astro
│   ├── Footer.astro
│   ├── Logo.astro
│   └── Section.astro
└── pages/
    └── [...slug].astro  # Dynamic page routing
```

---

## Brand Identity

### Name & Tagline

- **Name:** Everruns
- **Tagline:** "Durable AI. Unstoppable agents."
- **Meaning:** AI agents that **ever run** — continuous, uninterrupted, eternal execution

### Logo

Three interlocking rings (Borromean rings pattern) representing **Durability × Scalability × Reliability**.

**Logo Files (to be copied to new repo):**
```
public/
├── logo.svg        # Color version with navy-to-gold gradients
├── logo-mono.svg   # Black & white version
└── favicon.svg     # Use logo-mono.svg
```

**Color Logo SVG:**
```svg
<svg xmlns="http://www.w3.org/2000/svg"
     width="1024"
     height="1024"
     viewBox="0 0 512 512">

  <defs>
    <linearGradient id="gTop" gradientUnits="userSpaceOnUse"
      x1="256" y1="94" x2="256" y2="284">
      <stop offset="0.00" stop-color="#0A1636"/>
      <stop offset="0.70" stop-color="#0A1636"/>
      <stop offset="1.00" stop-color="#D4A43A"/>
    </linearGradient>

    <linearGradient id="gLeft" gradientUnits="userSpaceOnUse"
      x1="70" y1="374" x2="256" y2="284">
      <stop offset="0.00" stop-color="#081C3F"/>
      <stop offset="0.70" stop-color="#081C3F"/>
      <stop offset="1.00" stop-color="#D4A43A"/>
    </linearGradient>

    <linearGradient id="gRight" gradientUnits="userSpaceOnUse"
      x1="442" y1="374" x2="256" y2="284">
      <stop offset="0.00" stop-color="#0B1233"/>
      <stop offset="0.70" stop-color="#0B1233"/>
      <stop offset="1.00" stop-color="#D4A43A"/>
    </linearGradient>
  </defs>

  <g fill="none" stroke-width="18" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="256" cy="214" r="120" stroke="url(#gTop)"/>
    <circle cx="186" cy="309" r="120" stroke="url(#gLeft)"/>
    <circle cx="326" cy="309" r="120" stroke="url(#gRight)"/>
  </g>
</svg>
```

**Mono Logo SVG:**
```svg
<svg xmlns="http://www.w3.org/2000/svg"
     width="1024"
     height="1024"
     viewBox="0 0 512 512">

  <g fill="none" stroke="#0A0A0A" stroke-width="18"
     stroke-linecap="round" stroke-linejoin="round">
    <circle cx="256" cy="214" r="120"/>
    <circle cx="186" cy="309" r="120"/>
    <circle cx="326" cy="309" r="120"/>
  </g>
</svg>
```

### Color Palette

**Primary (Use sparingly — for accents only):**

| Name | Hex | Usage |
|------|-----|-------|
| Navy | `#0A1636` | Links, primary actions |
| Gold | `#D4A43A` | Highlights, success states |

**Grayscale (Primary UI colors):**

| Name | Hex | Usage |
|------|-----|-------|
| Obsidian | `#0A0A0A` | Primary text, headers |
| Charcoal | `#1A1A1A` | Dark backgrounds |
| Slate | `#404040` | Secondary text, borders |
| Silver | `#A0A0A0` | Muted text, captions |
| Smoke | `#F5F5F5` | Section backgrounds |
| White | `#FFFFFF` | Primary background |

### Typography

**Font:** Geist Sans / Geist Mono (free from Vercel)

```css
/* Install via npm: geist */

:root {
  --font-sans: 'Geist Sans', system-ui, sans-serif;
  --font-mono: 'Geist Mono', monospace;
}

h1 { font-weight: 600; font-size: 2.5rem; }
h2 { font-weight: 600; font-size: 2rem; }
h3 { font-weight: 500; font-size: 1.5rem; }
body { font-weight: 400; font-size: 1rem; line-height: 1.6; }
code { font-family: var(--font-mono); font-size: 0.875rem; }
```

### Voice & Tone

- **Confident**, not arrogant
- **Technical**, but accessible
- **Direct** — say it simply
- **Calm** — we handle chaos so you don't have to

---

## Page Design

### Design Principles

1. **Simple and clean** — no fancy colors, gradients only in logo
2. **Grayscale dominant** — content-first, minimal distraction
3. **Generous whitespace** — let content breathe
4. **Mobile-first** — responsive, works on all devices
5. **Fast** — minimal JavaScript, optimized images

### Layout Structure

```
┌─────────────────────────────────────────────────┐
│  [Logo]  Everruns              [GitHub] [Docs]  │  ← Header (sticky)
├─────────────────────────────────────────────────┤
│                                                 │
│              DURABLE AI.                        │
│          UNSTOPPABLE AGENTS.                    │
│                                                 │
│     Run AI agents that survive everything.      │
│                                                 │
│         [Get Started]  [GitHub →]               │
│                                                 │
├─────────────────────────────────────────────────┤
│                                                 │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐        │
│  │Durability│  │Real-time │  │Simple   │        │
│  │         │  │Streaming │  │API      │        │
│  └─────────┘  └─────────┘  └─────────┘        │
│                                                 │
├─────────────────────────────────────────────────┤
│                                                 │
│  How It Works                                   │
│  1. Create an agent                             │
│  2. Start a run                                 │
│  3. Agent survives any failure                  │
│                                                 │
├─────────────────────────────────────────────────┤
│                                                 │
│  Open Source                                    │
│  MIT licensed. Built with Rust and Temporal.    │
│                                                 │
├─────────────────────────────────────────────────┤
│                                                 │
│  [Logo]  Everruns                               │
│                                                 │
│  GitHub · Documentation · Contact               │
│                                                 │
│  ─────────────────────────────────────────────  │
│                                                 │
│           With love from Ukraine 🇺🇦            │
│                                                 │
│  © 2025 Everruns. MIT License.                  │
│                                                 │
└─────────────────────────────────────────────────┘
```

### Header

- **Left:** Logo (small, ~32px) + "Everruns" text
- **Right:** Navigation links
  - "GitHub" → https://github.com/ponyesteves/everruns
  - "Docs" → /docs (placeholder for now, link to GitHub README)
- **Behavior:** Sticky on scroll, subtle shadow when scrolled
- **Height:** 64px

### Hero Section

- **Background:** White
- **Content:**
  - Tagline: "Durable AI. Unstoppable agents." (H1, Obsidian)
  - Subtitle: "Run AI agents that survive crashes, restarts, and infrastructure failures." (Body, Slate)
  - CTAs:
    - Primary: "Get Started" → GitHub README
    - Secondary: "View on GitHub →" → GitHub repo

### Features Section

- **Background:** Smoke (`#F5F5F5`)
- **Layout:** 3-column grid (stacks on mobile)
- **Features:**

1. **Durable Execution**
   - Icon: Shield or lock (simple line icon)
   - "Every step persisted. Agents resume from where they left off."

2. **Real-time Streaming**
   - Icon: Signal or stream
   - "AG-UI protocol compatible. Watch agents think in real-time."

3. **Simple API**
   - Icon: Code brackets
   - "REST API with OpenAPI docs. Easy to integrate."

### How It Works Section

- **Background:** White
- **Layout:** Numbered steps, vertical on mobile, horizontal on desktop
- **Steps:**
  1. "Create an agent with a model and system prompt"
  2. "Start a run with your input message"
  3. "Agent executes durably — survives any failure"

### Open Source Section

- **Background:** Smoke
- **Content:**
  - "Open Source" (H2)
  - "MIT licensed. Built with Rust and Temporal."
  - "Star us on GitHub →" (link)

### Footer

- **Background:** Charcoal (`#1A1A1A`)
- **Text:** Silver (`#A0A0A0`)
- **Layout:**
  ```
  [Logo] Everruns

  Links: GitHub · Docs · Contact

  ────────────────────────────────

  With love from Ukraine 🇺🇦

  © 2025 Everruns. MIT License.
  ```

**Important:** The "With love from Ukraine" line must be present in the footer with the Ukrainian flag emoji.

---

## Initial Content (Markdown)

### Home Page (`src/content/pages/index.md`)

```markdown
---
title: Everruns - Durable AI Agent Platform
description: Run AI agents that survive crashes, restarts, and infrastructure failures.
---

## Durable AI. Unstoppable agents.

Run AI agents that survive crashes, restarts, and infrastructure failures.

Everruns is a durable execution platform for AI agents. Every step is persisted, so if anything goes wrong, your agent picks up right where it left off.

### Why Everruns?

**The problem:** AI agents are unreliable. A network blip, a service restart, a timeout — and your agent's work is lost.

**The solution:** Everruns makes agents unkillable. Built on Temporal, every step is persisted. Agents survive anything.

### Features

- **Durable Execution** — Every step persisted. Agents resume from where they left off.
- **Real-time Streaming** — AG-UI protocol compatible. Watch agents think in real-time.
- **Simple REST API** — OpenAPI documented. Easy to integrate with any stack.
- **Management UI** — Dashboard to manage agents, view runs, and chat.

### Open Source

Everruns is MIT licensed and open source.

- Built with Rust (API & Worker)
- Temporal for durable workflows
- PostgreSQL for storage
- Next.js Management UI

[View on GitHub →](https://github.com/ponyesteves/everruns)
```

---

## Technical Implementation

### Project Setup

```bash
# Create new Astro project
npm create astro@latest everruns-landing -- --template minimal

# Add required packages
npm install geist
npm install @astrojs/cloudflare

# Project structure
mkdir -p src/{content/pages,layouts,components}
mkdir -p public
```

### Astro Configuration (`astro.config.mjs`)

```javascript
import { defineConfig } from 'astro/config';
import cloudflare from '@astrojs/cloudflare';

export default defineConfig({
  output: 'static',
  adapter: cloudflare(),
  site: 'https://everruns.com',
});
```

### CSS Variables (`src/styles/global.css`)

```css
@import 'geist/font/sans';
@import 'geist/font/mono';

:root {
  /* Typography */
  --font-sans: 'Geist Sans', system-ui, -apple-system, sans-serif;
  --font-mono: 'Geist Mono', 'Fira Code', monospace;

  /* Colors - Grayscale */
  --color-obsidian: #0A0A0A;
  --color-charcoal: #1A1A1A;
  --color-slate: #404040;
  --color-silver: #A0A0A0;
  --color-smoke: #F5F5F5;
  --color-white: #FFFFFF;

  /* Colors - Accent */
  --color-navy: #0A1636;
  --color-gold: #D4A43A;

  /* Spacing */
  --space-xs: 0.25rem;
  --space-sm: 0.5rem;
  --space-md: 1rem;
  --space-lg: 2rem;
  --space-xl: 4rem;
  --space-2xl: 8rem;

  /* Layout */
  --max-width: 1200px;
  --header-height: 64px;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  font-family: var(--font-sans);
  font-size: 1rem;
  line-height: 1.6;
  color: var(--color-obsidian);
  background: var(--color-white);
}

a {
  color: var(--color-navy);
  text-decoration: none;
}

a:hover {
  text-decoration: underline;
}

code {
  font-family: var(--font-mono);
  font-size: 0.875rem;
  background: var(--color-smoke);
  padding: 0.125rem 0.375rem;
  border-radius: 4px;
}
```

### Cloudflare Pages Deployment

1. Connect GitHub repository to Cloudflare Pages
2. Configure build settings:
   - Framework preset: Astro
   - Build command: `npm run build`
   - Build output directory: `dist`
3. Set custom domain: `everruns.com`
4. Enable automatic deployments on push

---

## Links & Resources

| Resource | URL |
|----------|-----|
| GitHub Repository | https://github.com/ponyesteves/everruns |
| Astro Documentation | https://docs.astro.build |
| Cloudflare Pages | https://pages.cloudflare.com |
| Geist Font | https://vercel.com/font |

---

## Contact Information

Include in footer/contact page:

- **GitHub:** https://github.com/ponyesteves/everruns
- **Issues:** https://github.com/ponyesteves/everruns/issues

---

## Future Considerations

1. **API Documentation** — When ready, integrate Starlight (Astro's docs theme) for hosting OpenAPI/Swagger docs
2. **Blog** — Add content collection for blog posts
3. **Changelog** — Auto-generate from GitHub releases
4. **Status Page** — Link to external status page if needed

---

## Checklist for Implementation

- [ ] Create new repository (e.g., `everruns-landing`)
- [ ] Initialize Astro project with recommended config
- [ ] Copy logo files to `public/`
- [ ] Implement base layout with header and footer
- [ ] Create home page with all sections
- [ ] Add global styles following brand identity
- [ ] Set up Cloudflare Pages deployment
- [ ] Configure custom domain
- [ ] Test on mobile and desktop
- [ ] Verify "With love from Ukraine" is in footer
