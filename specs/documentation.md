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
├── index.md              # Landing page
├── getting-started/
│   └── introduction.md
├── features/
│   └── capabilities.md
├── sre/
│   ├── environment-variables.md
│   ├── admin-container.md
│   └── runbooks/
│       ├── authentication.md
│       ├── encryption-key-rotation.md
│       └── production-migrations.md
└── api/
    └── openapi.json      # Auto-generated OpenAPI spec
```

### Content Requirements

Each markdown file must include YAML frontmatter:

```yaml
---
title: Page Title
description: Brief description for SEO and search
---
```

### Design Requirements

Design follows the brand guidelines defined in [specs/brand.md](brand.md).

#### Color Scheme

| Theme | Background | Text | Accent |
|-------|------------|------|--------|
| Light | White/Smoke | Obsidian | Navy links, Gold hover |
| Dark | Navy | White | Gold links |

#### Typography

- **Font**: Geist Sans (body), Geist Mono (code)
- **Headings**: Weight 600 for H1-H3, 500 for H4-H6
- **Body**: Weight 400, line-height 1.6

#### Design Principles

1. **Simple and clean** — grayscale dominant
2. **Content-first** — minimal visual distraction
3. **Generous whitespace** — let content breathe
4. **Fast** — minimal external dependencies

### Build & Deployment

1. **Build Command**: `npm run build`
2. **Output Directory**: `dist/`
3. **Root Directory**: `apps/docs`
4. **Deployment Platform**: Cloudflare Pages (GitHub integration)
5. **CI Integration**: GitHub Actions workflow checks build on every PR

Cloudflare Pages dashboard configuration:
- Connect GitHub repository
- Set root directory: `apps/docs`
- Set build command: `npm run build`
- Set output directory: `dist`
- Node.js version: 20

### Development

```bash
# Install dependencies
cd apps/docs && npm install

# Local development
npm run dev

# Type checking
npm run check

# Build for production
npm run build
```

### API Reference Generation

API reference documentation is auto-generated from the OpenAPI specification using `starlight-openapi`.

#### Architecture

1. **Source of Truth**: OpenAPI spec generated from Rust code via `utoipa` derive macros
2. **Export Binary**: `export-openapi` binary generates spec without running full server
3. **Build-time Generation**: `starlight-openapi` plugin generates static HTML at build time
4. **Static Output**: No runtime dependencies - works on any static hosting (Cloudflare Pages)

#### Workflow

```bash
# 1. Generate OpenAPI spec (run when API changes)
./scripts/export-openapi.sh

# 2. Build docs (spec is read at build time)
cd apps/docs && npm run build
```

#### Files

| File | Purpose |
|------|---------|
| `docs/api/openapi.json` | Generated OpenAPI spec (committed to repo) |
| `scripts/export-openapi.sh` | Script to regenerate spec |
| `crates/control-plane/src/bin/export_openapi.rs` | Binary for spec generation |
| `crates/control-plane/src/openapi.rs` | Shared OpenAPI definition |

#### Starlight Integration

In `apps/docs/astro.config.mjs`:
```javascript
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";

export default defineConfig({
  integrations: [
    starlight({
      plugins: [
        starlightOpenAPI([{
          base: "api",
          label: "API Reference",
          schema: "../../docs/api/openapi.json",
        }]),
      ],
      sidebar: [
        // ... other items
        ...openAPISidebarGroups,  // Auto-generated from spec
      ],
    }),
  ],
});
```

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

### Future Enhancements

1. **Versioned Documentation**: Support for multiple documentation versions
2. **Search Analytics**: Track popular search queries to improve docs
3. **Changelog**: Auto-generate from GitHub releases
