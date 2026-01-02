# Documentation Site Specification

## Abstract

The Everruns documentation site provides user-facing documentation for operators and users of the platform. It is built with Astro Starlight and deployed to Cloudflare Pages at https://docs.everruns.com/.

## Requirements

### Site Structure

1. **Location**: `apps/docs/` in the monorepo
2. **Framework**: Astro with Starlight documentation theme
3. **Final URL**: https://docs.everruns.com/
4. **Content Format**: Markdown files with frontmatter

### Content Organization

1. **Getting Started**: Introduction and quickstart guides
2. **Features**: User-facing feature documentation (capabilities, etc.)
3. **SRE Guide**: Operational documentation
   - Environment variables reference
   - Admin container usage
   - Runbooks for common operations
4. **API Reference**: API documentation (future: generated from OpenAPI spec)

### Design Requirements

1. **Visual Consistency**: Design matches the main application
   - Brand colors from logo (dark blue: #0A1636, gold: #D4A43A)
   - Same logo SVG as the UI
   - Dark mode as default theme
2. **Fonts**: System UI fonts matching the application

### Content Migration

The original `docs/` folder at the repository root contains source documentation that is published through the docs site. Content from `docs/` should be kept in sync with `apps/docs/src/content/docs/`.

| Source Path | Published Path |
|-------------|----------------|
| `docs/sre/` | `apps/docs/src/content/docs/sre/` |
| `docs/features/` | `apps/docs/src/content/docs/features/` |

### Build & Deployment

1. **Build Command**: `npm run build`
2. **Output Directory**: `dist/`
3. **Deployment Platform**: Cloudflare Pages
4. **CI Integration**: GitHub Actions workflow checks build on every PR

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
