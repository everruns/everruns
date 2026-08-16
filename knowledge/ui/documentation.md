---
type: Specification
title: "Documentation Site Specification"
description: "Documentation site."
tags:
  - everruns
  - ui
---
# Documentation Site Specification

## Abstract

The Everruns documentation site provides user-facing documentation for operators and users of the platform. It is built with Astro Starlight and deployed to Cloudflare Pages at https://docs.everruns.com/.

## Requirements

### Site Structure

1. **Content Location**: `docs/` in the repository root
2. **Site Application**: `apps/docs/` (Astro Starlight)
3. **Content Symlink**: `apps/docs/src/content/docs` → `../../../../docs`
4. **Final URL**: https://docs.everruns.com/
5. **Content Format**: Markdown files with YAML frontmatter

### Content Organization

All documentation content lives in `docs/` at the repository root:

```
docs/
├── index.mdx             # Landing page (template: splash)
├── getting-started/
│   └── introduction.md
├── capabilities/
│   ├── index.md          # Capabilities overview + reference table
│   └── *.md              # Per-capability pages (file-system, bashkit-shell, etc.)
├── features/
│   └── capabilities.md
├── integrations/
│   └── daytona.md
├── providers/
│   ├── index.md          # Model providers overview
│   └── *.md              # Per-provider pages (openai, anthropic, bedrock, ...)
├── observability/
│   └── braintrust.md
├── sre/
│   ├── environment-variables.md
│   ├── admin-container.md
│   └── runbooks/
│       ├── authentication.md
│       └── encryption-key-rotation.md
└── api/
    └── openapi.json      # Auto-generated OpenAPI spec
```

### Navigation

The docs site uses `starlight-sidebar-topics` for section-based navigation,
rendered as horizontal tabs below the header on desktop:

| Tab | Content |
|-----|---------|
| Get Started | Getting Started guides + Features |
| Integrations | Integrations + Providers + Observability + Ecosystem |
| Capabilities | Per-capability reference pages (tools, examples, use cases) |
| Operations | SRE Guide + Runbooks |
| Reference | Event Reference + API Reference (OpenAPI) |

Custom `Header.astro` override renders topics as a fixed tab bar below the
main header. Sidebar topic list is hidden on desktop (visible on mobile).

The Reference tab must remain active for all `/api/` pages, including nested
OpenAPI-generated routes such as `/api/operations/*` and `/api/operations/tags/*`.

### Content Requirements

`docs/` is public product documentation. It must not contain research proposals,
internal proposals, draft specs, temporary investigations, or scratch analysis.
Durable internal design intent belongs in `knowledge/`; temporary research belongs
outside the repo.

Each markdown file must include YAML frontmatter:

```yaml
---
title: Page Title
description: Brief description for SEO and search
hero: ../images/section/visual.png  # Optional: hero image for social card
---
```

- `title` and `description` are required
- `hero` is optional — relative path to an image that will be composited into the page's OG social card (see [Social Card Images](#social-card-images-og-images))

#### Notebook-Backed Tutorials

Notebook tutorials use a checked-in `.ipynb` file as the source of truth and a hand-authored MDX wrapper for page metadata.

- The wrapper page lives in `docs/**/index.mdx`
- The wrapper frontmatter owns durable page metadata such as `title`, `description`, optional `slug`, and cookbook metadata like `published`, `topics`, and `github`
- Notebook-backed wrappers must set `notebook: ./relative-path.ipynb` in frontmatter
- The wrapper must not duplicate notebook cell content inline; it should stay small and only provide durable metadata plus the renderer component
- `apps/docs/scripts/render-notebooks.mjs` pre-renders referenced notebooks into static HTML before Astro runs
- The pre-render step also copies raw notebooks into the built site as downloadable assets under `/notebooks/**`
- Notebook-backed pages should read like cookbook articles, with the notebook markdown rendered as the main page content rather than as an embedded viewer widget
- All notebook pages must render as static HTML in the final build for search indexing and fast delivery
- Every `.ipynb` under `docs/tutorials/` must be referenced by exactly one MDX wrapper
- The notebook source should default to `https://app.everruns.com/api`; local execution should override `EVERRUNS_API_URL` instead of editing the notebook
- CI must execute notebook-backed tutorials against a local Everruns dev-mode API using `EVERRUNS_API_URL=http://127.0.0.1:9301/api`, `EVERRUNS_API_KEY=dev`, and `EVERRUNS_NOTEBOOK_USE_LLMSIM=1`

### Design Requirements

Design follows the brand guidelines defined in [knowledge/ui/brand.md](brand.md) (colors, typography, visual principles).

### Screenshots

Screenshots must be captured from the real running product — never faked,
mocked, or staged.

- **Do not** hand-author HTML/CSS that imitates the UI, reconstruct a
  component's markup, paint a screenshot in a design tool, or edit an image so
  it shows behavior the product does not actually produce. A picture that looks
  like the product but was not produced by it is a fabrication, even when it is
  visually faithful.
- **Do** capture the actual application, or the actual React component from
  `apps/ui` rendered by the real app (e.g. a throwaway route that mounts the
  real component with sample props, screenshotted via the pre-installed
  Chromium). The image must reflect the real component's styling, layout, and
  behavior.
- Any temporary route, fixture, or harness added only to capture a screenshot
  must be removed before committing; commit the image, not the scaffolding.
- Prefer representative sample data over real user/org data, and never include
  secrets, tokens, or private content in a captured image.

This rule exists because docs screenshots are load-bearing evidence of how the
product behaves; a fabricated one silently misleads readers.

### Build & Deployment

1. **Build Command**: `pnpm run build`
2. **Output Directory**: `dist/`
3. **Root Directory**: `apps/docs`
4. **Deployment Platform**: Cloudflare Pages (GitHub integration)
5. **CI Integration**: GitHub Actions workflow checks build on every PR

Cloudflare Pages dashboard configuration:
- Connect GitHub repository
- Set root directory: `apps/docs`
- Set build command: `pnpm run build` (Pages auto-installs from `pnpm-lock.yaml` before running the build command)
- Set output directory: `dist`
- Pin Node.js from `apps/docs/.node-version` (currently `22.16.0`) so Pages builds do not depend on dashboard defaults; any override must stay at `20.19.1+` or `22.12+` to satisfy Astro 6 toolchain requirements
- Pnpm is selected automatically when Cloudflare Pages sees `pnpm-lock.yaml`; the `packageManager` field in `apps/docs/package.json` pins the exact version via corepack

The docs homepage advertises agent-discoverable API resources with RFC 8288
`Link` response headers from `apps/docs/public/_headers`. The header points to
the docs API reference.

### Development

```bash
# Install dependencies
cd apps/docs && pnpm install

# Local development
pnpm run dev

# Type checking
pnpm run check

# Build for production
pnpm run build
```

### API Reference Generation

API reference documentation is auto-generated from the OpenAPI specification using `starlight-openapi`.

#### Architecture

1. **Source of Truth**: OpenAPI spec generated from Rust code via `utoipa` derive macros
2. **Export Binary**: `export-openapi` binary generates spec without running full server
3. **Build-time Generation**: `starlight-openapi` plugin generates static HTML at build time
4. **Static Output**: No runtime dependencies - works on any static hosting (Cloudflare Pages)
5. **Topic Navigation**: OpenAPI pages are excluded from `starlight-sidebar-topics` sidebar ownership, while the custom header still marks the Reference tab active for `/api/**`

#### Workflow

```bash
# 1. Generate OpenAPI spec (run when API changes)
./scripts/export-openapi.sh

# 2. Build docs (spec is read at build time)
cd apps/docs && pnpm run build
```

#### Files

| File | Purpose |
|------|---------|
| `docs/api/openapi.json` | Generated OpenAPI spec (committed to repo) |
| `scripts/export-openapi.sh` | Script to regenerate spec |
| `crates/server/src/bin/export_openapi.rs` | Binary for spec generation |
| `crates/server/src/openapi.rs` | Shared OpenAPI definition |

#### Starlight Integration

In `apps/docs/astro.config.mjs`:
See `apps/docs/astro.config.mjs` for the full sidebar topics and OpenAPI
plugin configuration.

#### CI/CD Integration

The OpenAPI spec should be regenerated and committed when API endpoints change:

1. Developer modifies API endpoints or schemas
2. Run `./scripts/export-openapi.sh` to update spec
3. Commit `docs/api/openapi.json` with API changes
4. Docs build in CI reads spec and generates API reference pages

**Freshness Check**: CI includes an `openapi-check` job that:
- Generates a fresh spec from current code
- Compares with committed `docs/api/openapi.json`
- Fails the build if they differ

This ensures developers cannot forget to regenerate the spec after API changes.

### SEO Requirements

The docs site must maintain good SEO hygiene to ensure discoverability.

#### Meta Descriptions

Every page must have a `<meta name="description">` tag.

- **Content pages**: Set `description` in YAML frontmatter (required for all `docs/*.md` files)
- **API pages**: Auto-generated via route middleware (`apps/docs/src/routeData.ts`)
- **Fallback**: Starlight `description` config provides a site-level fallback
- **Target length**: Aim for roughly 50-160 characters for hand-authored content page descriptions

#### Page Titles

Page titles (rendered as `<title>Title | Everruns</title>`) must not exceed 70 characters total.

- Starlight appends ` | Everruns` (12 chars), so page titles must be ≤57 chars
- For API pages, route middleware strips the `METHOD /path - ` prefix from OpenAPI summaries to shorten titles
- When writing OpenAPI `summary` doc comments in Rust, the description after the `METHOD /path - ` prefix should be ≤57 chars

#### Images

All `<img>` tags must have an `alt` attribute.

- The Starlight logo config must include `alt: "Everruns"`
- Markdown images: `![descriptive alt text](path/to/image.png)`
- HTML images: `<img src="..." alt="descriptive text" />`

#### Links

No internal links should produce 404 errors.

- Content pages under `docs/` should link using root-relative paths matching the sidebar structure (e.g. `/event-reference/`, not `/features/event-reference`)
- Links to internal knowledge (`knowledge/**/*.md`) should use absolute GitHub URLs since the bundle is not published as docs pages
- The `starlight-openapi` plugin does not generate individual schema pages — do not link to `/api/schemas/{SchemaName}`

#### Social Card Images (OG Images)

Every page gets a per-page Open Graph image for rich link previews on Twitter, Slack, Discord, etc. Images are generated at prebuild time by `apps/docs/scripts/generate-og-image.mjs` and output to `public/og/`.

**Three card types:**

| Type | Path | Content |
|------|------|---------|
| API operation | `public/og/api/operations/{operationId}.png` | Method badge, path, description, curl example |
| Doc page | `public/og/{slug}.png` | Title, breadcrumb, description |
| Fallback | `public/og-image.png` | Generic Everruns branded card |

**Hero images on doc pages:**

Doc pages can include a hero image (screenshot, diagram, logo) that gets composited into the right half of the OG card. Detection order:

1. **Frontmatter `hero` field** (preferred): `hero: ./my-feature-screenshot.png` (relative to the page; co-located with it per the placement rule below)
2. **First markdown image**: The first `![...](path)` in the page body (e.g. `![Daytona Integration](./daytona.png)`)

When a hero is detected, the card layout shifts to a split view: text on the left, hero image on the right. Pages without a hero image use the full-width text layout.

**Adding hero images to new pages:**

When creating or editing a doc page that has a visual asset (integration logo, architecture diagram, UI screenshot), add it as a hero so the social card is richer:

```yaml
---
title: My Feature
description: What this feature does
hero: ./my-feature-screenshot.png
---
```

(The legacy form `hero: ../images/features/my-feature-screenshot.png` still resolves for grandfathered assets under `docs/images/{section}/`, but new pages should follow the co-located form above.)

Place new hero images (and any other diagrams or screenshots used by a single page) in the **same directory as the page that embeds them**, and reference them with relative paths (`hero: ./my-feature-screenshot.png`). Existing assets under `docs/images/{section}/` continue to work and are not migrated en masse. See `knowledge/docs/diagrams.md` for the colocation rule applied to diagrams.

**Generated files are gitignored** — `public/og/` and `public/og-image.png` are rebuilt on every `pnpm run build` (via the `prebuild` script). Only the generation script and source images are committed.

#### Route Middleware (`apps/docs/src/routeData.ts`)

SEO improvements for auto-generated pages are handled by Starlight route middleware:

1. Strips `METHOD /path - ` prefix from API operation titles for shorter `<title>` tags
2. Generates per-page meta descriptions for API reference pages that lack frontmatter descriptions
3. Updates `og:title` and `og:description` to match
4. Sets per-page `og:image` pointing to the pre-generated social card PNG for the current page

### Sitemap Requirements

The docs site must ship a single `sitemap.xml` file at the site root.

1. `robots.txt` must reference `https://docs.everruns.com/sitemap.xml`
2. `sitemap.xml` must include both hand-authored docs pages and OpenAPI-generated API reference pages
3. Every `<url>` entry must include a `<lastmod>` value
4. `apps/docs/integrations/sitemap-enhance.mjs` is responsible for post-processing Astro's generated sitemap into the final `sitemap.xml`
5. `lastmod` currently uses the docs build date rather than git history so builds remain deterministic on Cloudflare Pages shallow clones

### Diagram Rendering

Diagrams are hand-authored SVGs following `knowledge/docs/diagrams.md`. Each SVG has a co-located `.mmd` (Mermaid) source-of-truth file.

1. New SVGs (and their `.mmd` siblings) **must** live in the same directory as the markdown page that embeds them, and are referenced via `![alt](./<name>.svg)`. See `knowledge/docs/diagrams.md` for the full placement rule.
2. Legacy diagrams under `docs/images/<category>/` remain in place and continue to work; they are not migrated en masse.
3. No client-side rendering library is needed — SVGs are static assets processed by Astro's image pipeline
4. The Mermaid `.mmd` files are source-of-truth for diagram content but are not rendered at build time
