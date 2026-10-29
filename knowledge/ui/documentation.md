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

The Everruns documentation site provides user-facing documentation for operators and users of the platform. It is built with Astro Starlight and served at https://everruns.com/docs/ by the `everruns-docs` Cloudflare Worker.

## Requirements

### Integration landing pages

The [Slack landing page](../../docs/integrations/slack.mdx) introduces building
and publishing an Agent with realistic English examples and the public demo.
It is distinct from the detailed [Slack reference](../../docs/capabilities/slack.md).
The integration route is a page, not an alias to the capability reference.
Examples reconstructed or translated from captures must say so; raw recordings
and unrelated messages stay out of published assets.

### Site Structure

1. **Content Location**: `docs/` in the repository root
2. **Site Application**: `apps/docs/` (Astro Starlight)
3. **Content Symlink**: `apps/docs/src/content/docs` → `../../../../docs`
4. **Final URL**: https://everruns.com/docs/
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
| Start here | The chooser between Framework, self-hosted, and Everruns Cloud; the Docker Compose and Cloud quickstarts; concepts; tutorials |
| Framework | The in-process Rust Framework |
| Platform | What self-hosted and Everruns Cloud share: features, how-to guides, advanced topics, explanation; plus self-hosted operations |
| Reference | Event reference, harness and capability catalog, integrations and providers, dev-grade-only experiments, API reference (OpenAPI) |

Why these four: the first question a reader has is which Everruns they are
using. The three ways of running it appear in the same order and with the same
names everywhere (README, llms.txt, chooser, badges): Framework, Self-hosted,
Everruns Cloud. "Everruns Cloud" is the only name for app.everruns.com.

Capabilities and integrations registered only at the development deployment
grade sit in one "Experimental (dev grade)" group rather than beside the
supported ones, because Everruns Cloud runs at production grade and never
offers them.

The layout replaces Starlight's fixed frame (`PageFrame.astro`): the
everruns.com site header scrolls away, and a 48px docs bar (`Header.astro`:
"Docs", the topic tabs, search, a light/dark/system switch) sticks to the top.
Once the site header is out of view the bar collapses: the logo and an Open
Cloud button slide in, so the bar alone still says which site this is. Below
it, one 1200px column holds the sidebar, the article (720px) and the "On this
page" column, which folds into a sticky dropdown below 1200px; the sidebar
becomes a popover menu below 960px. The sidebar topic list shows only in that
menu, since the tabs carry it on wider screens. Slate tokens and Geist live in
`apps/docs/src/styles/slate.css`; code blocks are dark in both themes
(`apps/docs/src/code-theme.json`).

The Reference tab must remain active for all `/api/` pages, including nested
OpenAPI-generated routes such as `/api/operations/*` and `/api/operations/tags/*`.

### API reference

The reference leads with the caller-facing Agent API, followed by collapsed
management domains. Its group landing pages explain what a caller can do;
operation pages place schemas beside request and response examples. The
existing operation and tag URLs remain durable. Every new canonical group
and operation page must be in the sitemap and have a Markdown twin; any
retired documentation URL needs a permanent redirect through the docs Worker.

Grouping, readable labels, support, audience, availability and migration
guidance come from the exported OpenAPI document, owned by the
[documentation modifier](../../crates/server/src/openapi/documentation.rs).
The renderer must not promote an unclassified operation to supported by
default. Support describes compatibility; audience describes who the API is
for, and neither changes authorization. Current public APIs are Preview
unless explicitly Experimental; Stable is reserved for an announced
compatibility commitment. Deprecated routes stay discoverable and name a
successor when one exists; an unannounced removal date must not be invented.

Use the existing schema renderer for references, alternatives, examples,
callbacks and security requirements. Keep the API navigation focused on the
reference and preserve the site's Slate styling. Endpoint search loads a
static index on demand and bounds displayed matches so browsing never becomes
another full operation dump. The [build verifier](../../apps/docs/scripts/verify-api-reference.mjs)
checks route, sitemap, Markdown, migration and search coverage.

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
- `hero` is optional, relative path to an image that will be composited into the page's OG social card (see [Social Card Images](#social-card-images-og-images))
- `appliesTo` is optional: any of `framework`, `platform`, `cloud`. `PageTitle.astro` renders it as an "Applies to" badge row under the title. Set it on capability, feature, and how-to pages, and only after checking availability in code: `framework` means the default Framework capability registry or an `everruns` Cargo feature; `platform` and `cloud` share the server registry, except that plugins behind a dev-grade feature flag are never on Cloud. A page without the field shows no row, which is better than a wrong one

Unavailable targets keep muted gray labels and borders without strikethroughs; the
badge row indicates applicability without making other products look deprecated.

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

#### Checked Catalogs and Snippets

Hand-written catalogs and snippets drifted from the code because nothing
compared them. Each now has a source of truth CI compares it against:

- The capability index, the built-in harness pages, the event reference, and the
  environment-variable summary are checked by `scripts/check_docs_catalogs.py`.
  Capabilities come from `docs/api/capability-catalog.json`, a registry snapshot
  that a Rust test keeps equal to what the server registers at each grade; the
  check itself needs no Rust build, so a docs-only edit is checked too.
- Every Python block is checked against the published SDK (names, methods,
  arguments), and complete programs run against llmsim in the notebook step.
  Most guide blocks are fragments that need earlier steps, so they are checked,
  not run.
- Framework Rust blocks compile as doctests (see
  `knowledge/framework/documentation-and-examples.md`).

### Design Requirements

Design follows the brand guidelines defined in [knowledge/ui/brand.md](brand.md) (colors, typography, visual principles).

The docs share everruns.com with the landing site, so they wear its header and
footer rather than a look of their own: a reader moving between `/` and
`/docs/` should not notice a second site. Landing (everruns/landing) owns that
chrome and publishes it under `/site-chrome/`; `apps/docs/scripts/sync-site-chrome.mjs`
snapshots it into `apps/docs/src/site-chrome/`, committed so a docs build never
depends on landing being up, and the daily `docs-deploy.yml` run fails when the
snapshot drifts. Change the chrome in landing, then resync here.

Search is one box for the whole domain: the docs Pagefind index merges the
landing index at `everruns.com/pagefind/` (built by landing from its
`llms-full.txt`), weighted up so a product's own page can surface among the
many docs pages.

### Screenshots

Screenshots must be captured from the real running product, never faked,
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

This rule exists because docs screenshots are essential evidence of how the
product behaves; a fabricated one silently misleads readers.

### Build & Deployment

1. **Build Command**: `pnpm run build`
2. **Output Directory**: `dist/`
3. **Root Directory**: `apps/docs`
4. **Deployment Platform**: Cloudflare Worker `everruns-docs` with static assets
   (`apps/docs/wrangler.jsonc`, `apps/docs/worker/index.mjs`), deployed by
   `.github/workflows/docs-deploy.yml`
5. **CI Integration**: `ci.yml` checks the build and the Worker on every PR;
   `docs-deploy.yml` deploys from main only, because it holds the Doppler token.
   A preview is a manual `wrangler versions upload --preview-alias <name>`,
   checked with `scripts/check-live.mjs --site <preview>/docs`

#### Serving under everruns.com/docs

Decision (2026-10-12): the docs moved from docs.everruns.com to a subfolder of
everruns.com, so links to the docs count toward one domain and readers stay on
one site. The landing site (`everruns/landing`) owns everruns.com as a Custom
Domain on its own Worker; the docs Worker is attached to the route
`everruns.com/docs*`, which Cloudflare matches before the Custom Domain. The two
sites build and deploy independently and no request passes through both.

1. `site` and `base` live in `apps/docs/site.config.mjs`. Astro adds the base
   only to URLs it generates, so content keeps site-root links (`/framework/`)
   and `plugins/rehype-base-links.ts` prefixes them at build time; components
   use `src/lib/site.ts` helpers, never a hard-coded `/...` path
2. `scripts/stage-worker-assets.mjs` copies `dist/` to `dist-worker/docs/`,
   because the asset server answers by public path, and turns every
   `redirects` entry into a real 301 in `_redirects`
3. The Worker negotiates content like the landing site: `Accept:
   text/markdown` (or a known AI user agent, never a search crawler) on a page
   URL returns its `index.md` twin, with `Vary: Accept, User-Agent`. Every page
   response carries `Link` headers for `llms.txt` and the API reference
4. The docs Worker also owns the route `docs.everruns.com/*` and 301s every
   path, query included, to the same path under https://everruns.com/docs. A
   Worker route rather than a zone redirect rule, because the deploy token
   manages Worker routes and the redirect stays tested in the repo. It stays
   forever: published crate docs, plugins and old posts link the old host.
   A route fires only on a proxied hostname, so docs.everruns.com keeps a
   proxied placeholder DNS record (`AAAA 100::`, discard address, added
   2026-10-13 after the old Pages project and its CNAME were deleted).
   Deleting that record takes every old link down
5. `apps/docs/scripts/check-live.mjs` checks every sitemap page and its
   Markdown twin, the agent files, content negotiation and, with `--old`, each
   old URL's redirect. It runs after every deploy and daily
6. `scripts/lib/check-docs-host.sh` (pre-push and CI) fails on any new link
   to docs.everruns.com outside the files about the move itself; the
   maintenance skill sweeps the sibling repositories for the same

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
4. **Static Output**: No runtime dependencies - works on any static hosting
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
| `crates/server/src/openapi/mod.rs` | Shared OpenAPI definition |

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

A description says what the page covers, in one factual sentence. It is not ad copy: no second
sentence restating the first as a benefit ("Agents can run shell commands safely with..."), no
unverifiable adjectives (secure, powerful, comprehensive, recommended), and no
"Explore/Discover/Learn how" openers. Crawlers and the OG card both render it verbatim, so padding
costs the reader twice.

#### Page Titles

Page titles (rendered as `<title>Title | Everruns</title>`) must not exceed 70 characters total.

- Starlight appends ` | Everruns` (12 chars), so page titles should be ≤57 chars;
  `routeData.ts` drops the suffix from a title that would still run over, and
  `verify-seo.mjs` fails the build on any title over 70
- API pages are titled by the OpenAPI summary, which the server keeps to ≤58
  chars (`MAX_SUMMARY` in `crates/server/src/openapi/documentation.rs`, enforced
  by its test). The server strips a `METHOD /path - ` prefix, takes the first
  sentence, and moves the full text into the operation description, which the
  page shows as its lede. A `#[command]` whose first sentence is still too long
  sets `summary = "..."`; a hand-written handler puts a short first doc line

A title is the page's name, not a keyword string. Prefer the name a reader would say out loud
("Daytona"), not the search phrase it might match ("Daytona Cloud Sandbox Capability for Agents").
A `sidebar.label` that differs from the title only to hide a longer title from the sidebar is a
sign the title is wrong; set `sidebar.label` only when the sidebar genuinely needs a shorter or
disambiguated form.

#### Images

All `<img>` tags must have an `alt` attribute.

- The Starlight logo config must include `alt: "Everruns"`
- Markdown images: `![descriptive alt text](path/to/image.png)`
- HTML images: `<img src="..." alt="descriptive text" />`

#### Links

No internal links should produce 404 errors.

- Content pages under `docs/` should link using root-relative paths matching the sidebar structure (e.g. `/event-reference/`, not `/features/event-reference`)
- Links to internal knowledge (`knowledge/**/*.md`) should use absolute GitHub URLs since the bundle is not published as docs pages
- The `starlight-openapi` plugin does not generate individual schema pages, do not link to `/api/schemas/{SchemaName}`

#### Social Card Images (OG Images)

Every page gets a per-page Open Graph image for rich link previews on Twitter, Slack, Discord, etc. Images are generated at prebuild time by `apps/docs/scripts/generate-og-image.mjs` and output to `public/og/`.

**Card types:**

| Type | Path | Content |
|------|------|---------|
| API operation | `public/og/api/operations/{operationId}.png` | Method badge, path, description, curl example |
| Doc page | `public/og/{slug}.png` | Title, breadcrumb, description |
| Changelog index | `public/og/changelog.png` | The five latest releases |
| Changelog entry | `public/og/changelog/{folder}.png` | Date, products, title, description |
| Changelog product page | `public/og/changelog/{product}.png` | That product's five latest entries |
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

**Generated files are gitignored**: `public/og/` and `public/og-image.png` are rebuilt on every `pnpm run build` (via the `prebuild` script). Only the generation script and source images are committed.

#### Route Middleware (`apps/docs/src/routeData.ts`)

SEO improvements for auto-generated pages are handled by Starlight route middleware:

1. Strips `METHOD /path - ` prefix from API operation titles for shorter `<title>` tags
2. Generates per-page meta descriptions for API reference pages that lack frontmatter descriptions
3. Updates `og:title` and `og:description` to match
4. Sets per-page `og:image` pointing to the pre-generated social card PNG for the current page,
   and a matching per-page `og:image:alt` (the global alt in `astro.config.mjs` describes the
   fallback card only, so it does not fit a per-page card)
5. Leaves `og:image` alone for routes with no generated card — `generate-og-image.mjs` skips
   `docs/api/`, so the `api` overview and the schema and tag pages fall back to `public/og-image.png`

6. Names each API tag page after its tag ("MCP Servers API"), since `starlight-openapi` titles
   them all "Overview", and marks a `_legacy` operation's title "(legacy)" so it does not
   duplicate its successor's
7. Keeps API operation descriptions within a search snippet: the reference suffix goes on only
   where it fits in 160 characters

Generated descriptions are held to the same house style as hand-authored ones above.

#### Analytics

Every docs page loads the same Google tag as everruns.com (`apps/docs/src/google-tag.js`,
inlined into the head) and fires the landing site's event names
(`cloud_signup_click`, `github_click`, `subscribe`), so conversions from docs readers are
counted with the landing site's. Keep the tag id and event names in step with
everruns/landing `src/components/GoogleTag.astro`.

#### Build Verification

`apps/docs/scripts/verify-seo.mjs` runs after every build and fails it when a page has no
canonical or `og:url` naming its own address, a `<title>` another page shares, a description
outside 50-170 characters, an `og:image` the build does not contain, or other than one `<h1>`, or
when `404.html` is indexable. Two pages on one subject (a feature page and its capability page)
keep distinct `<title>`s through a frontmatter `head` title override.

The docs Worker answers a page URL without its trailing slash with a 301 to the slashed URL;
Workers static assets would answer 307, which tells search engines the move is temporary.

### Sitemap Requirements

The docs site must ship a single `sitemap.xml` file at the docs root.

1. The landing site's `robots.txt` (everruns.com owns `/robots.txt`) must reference the docs sitemap index, `https://everruns.com/docs/sitemap-index.xml`
2. `sitemap.xml` must include both hand-authored docs pages and OpenAPI-generated API reference pages
3. Every `<url>` entry must include a `<lastmod>` value
4. `apps/docs/integrations/sitemap-enhance.mjs` is responsible for post-processing Astro's generated sitemap into the final `sitemap.xml`
5. `lastmod` currently uses the docs build date rather than git history so builds remain deterministic on shallow CI clones
6. Explicitly redirected announcements are excluded from the sitemap; their successors remain canonical. The Worker and sitemap build share the [redirect rules](../../apps/docs/redirects.mjs), so live checks continue to require 200 responses for every listed page.

### Agent-Readable Output Requirements

Agents read the docs as text, not HTML. The site must serve that text itself
rather than leave crawlers to reconstruct it, and the text must be usable
without the site next to it.

1. `/llms.txt` must be an index, not a pointer to two dumps: the three ways to
   run Everruns (mirroring the README's "Choose how you run Everruns" table),
   one link per documentation set, and the machine-readable surfaces the sets
   leave out. `starlight-llms-txt` derives no page index from the sidebar, so
   the index exists only as far as `customSets` and `optionalLinks` describe it
2. Every documentation set in `astro.config.mjs` must correspond to a sidebar
   topic or a group within one, so a reader with a narrow question can take one set — the complete
   text is around 250k tokens
3. Every page in every text output must carry a `Source:` line with its
   canonical URL, and links must be absolute: the outputs are read away from
   the site, where a root-relative path resolves against the wrong host
4. `llms-small.txt` must be meaningfully smaller than `llms-full.txt` (at most
   75%). Whitespace collapse alone is not an abridgement; `excludeSmall` drops
   the vendor- and operator-specific long tails, which stay reachable in
   `llms-full.txt` and in the sets
5. The REST API is excluded from the text outputs because the OpenAPI schema is
   the better machine-readable form — so the schema must actually be served, at
   `/api/openapi.json` (`apps/docs/scripts/copy-openapi.mjs`)
6. The landing site's `robots.txt` must allow the AI crawlers that respect it
7. Every built documentation page at `<page>/` must also be available as
   `<page>/index.md`; the site root is available as `/index.md`. Per-page
   Markdown must carry the same canonical `Source:` line and absolute links as
   the whole-corpus outputs
8. `apps/docs/scripts/verify-llms.mjs` enforces all of the above at postbuild,
   including one-to-one Markdown coverage for built HTML pages and that each
   `Source:` URL resolves to a built page
9. Requirements 1-4 and 7 need behavior the upstream plugin does not have; the delta
   lives in `apps/docs/patches/starlight-llms-txt@0.11.0.patch` and must be
   re-cut when the plugin is upgraded

### Diagram Rendering

Diagrams are hand-authored SVGs following `knowledge/docs/diagrams.md`. Each SVG has a co-located `.mmd` (Mermaid) source-of-truth file.

1. New SVGs (and their `.mmd` siblings) **must** live in the same directory as the markdown page that embeds them, and are referenced via `![alt](./<name>.svg)`. See `knowledge/docs/diagrams.md` for the full placement rule.
2. Legacy diagrams under `docs/images/<category>/` remain in place and continue to work; they are not migrated en masse.
3. No client-side rendering library is needed, SVGs are static assets processed by Astro's image pipeline
4. The Mermaid `.mmd` files are source-of-truth for diagram content but are not rendered at build time
