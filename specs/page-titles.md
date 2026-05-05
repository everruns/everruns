# Page Titles

## Intent

Every page in the web UI must set a meaningful `<title>`. Browser tabs, history,
search results, and screen readers all rely on it. A single static
"Everruns - AI Agent Management" across every route is not acceptable.

## Format

```
<Specific> · <Section> · Everruns
```

- Separator: middle dot ` · ` (U+00B7) with single spaces
- App suffix: always `Everruns` at the end
- Order: most specific first, broadest last
- Two to four segments total. Drop empty segments.

Examples:

- `Agents · Everruns`
- `New Agent · Agents · Everruns`
- `My Customer Bot · Agent · Everruns`
- `Edit · My Customer Bot · Agent · Everruns`
- `API Keys · Settings · Everruns`
- `Schedules · Durable · Everruns`
- `Trajectory · Friday triage · Session · Everruns`
- `Sign in · Everruns`

## Display name vs name

For entities exposing both `name` and `display_name`, the title MUST use
`getDisplayName(entity)` (`apps/ui/src/lib/entity-lifecycle.ts`). The slug
`name` is for URLs, never the title.

While the entity is still loading or could not be resolved, fall back to the
entity kind alone (`Agent · Everruns`) rather than showing the URL slug or an
empty segment. Once the data resolves the title updates in place.

## Implementation

Helper: `apps/ui/src/lib/page-title.ts`

- `APP_NAME = "Everruns"`
- `TITLE_SEPARATOR = " · "`
- `formatPageTitle(...parts)` joins truthy parts with the separator and appends
  `APP_NAME`.

Hook: `apps/ui/src/hooks/use-page-title.ts`

- `usePageTitle(...parts)` sets `document.title` via `useEffect` and restores
  the previous title on unmount. Safe to call from any client component.
- Pass `null` / `undefined` for segments that are still loading; the hook skips
  them and re-runs once they resolve.

Server pages with no client interactivity SHOULD export Next.js `metadata`
instead of using the hook. Mixing both is fine: the hook overrides static
metadata once mounted.

## Coverage requirement

Every route under `apps/ui/src/app/` MUST set a title:

- List/index pages: section name (e.g. `Agents`)
- Create pages: `New <Kind>` or `Create <Kind>`
- Edit pages: prefix `Edit · ` to the detail title
- Detail pages: `<displayName> · <Kind>`
- Sub-tabs (sessions, settings, durable): `<Sub> · <Parent specific or kind>`
- Auth pages: action verb only (`Sign in`, `Sign up`)
- Dev / showcase pages: `<Component> · Dev`

When adding a new route, the page title is part of the work — pages without a
title should fail review.

## Non-goals

- No SEO meta description churn — we update only the title.
- No i18n yet; titles are English. When localization arrives, the format and
  separator stay; segments translate.
