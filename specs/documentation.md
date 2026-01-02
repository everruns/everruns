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
    └── overview.md
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

1. **Visual Consistency**: Design matches the main application
   - Brand colors from logo (dark blue: #0A1636, gold: #D4A43A)
   - Same logo SVG as the UI (`apps/docs/src/assets/logo.svg`)
   - Dark mode as default theme
2. **Fonts**: System UI fonts matching the application

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

### Future Enhancements

1. **API Reference Generation**: Generate API documentation from OpenAPI spec
2. **Versioned Documentation**: Support for multiple documentation versions
3. **Search Analytics**: Track popular search queries to improve docs
