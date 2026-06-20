# Everruns icon for lobehub/lobe-icons

A ready-to-submit Everruns brand icon for [`lobehub/lobe-icons`](https://github.com/lobehub/lobe-icons),
built to match the repo's contribution conventions (see [issue #107](https://github.com/lobehub/lobe-icons/issues/107)).

The artwork is the Everruns mark — three interlocking Borromean rings
(Durability × Scalability × Reliability), navy → gold gradient converging at the
center — taken directly from `logo.svg` / `apps/ui/public/logo.svg` and fitted to
lobe-icons' 24×24 viewBox.

## What's here

```
src/Everruns/                 # drop straight into lobe-icons' src/
├── index.ts                  # compound export (Mono + Color/Text/Combine/Avatar)
├── style.ts                  # TITLE, COLOR_PRIMARY, combine + avatar constants
├── index.md                  # dumi docs page (Provider group)
└── components/
    ├── Mono.tsx              # 24×24, stroke="currentColor"
    ├── Color.tsx             # 24×24, navy→gold gradients (useFillIds)
    ├── Text.tsx              # "Everruns" wordmark, Geist, height 24
    ├── Combine.tsx           # icon + wordmark (mono/color)
    └── Avatar.tsx            # navy bg, gold rings

everruns.patch                # same change as a git patch incl. src/icons.ts export
preview/                      # standalone SVGs + rendered PNGs (verification only)
```

`src/icons.ts` gets one alphabetically-placed line:

```ts
export { default as Everruns, type CompoundedIcon as EverrunsProps } from './Everruns';
```

The `static-svg` / `static-png` / `static-webp` packages and the table of
contents are generated automatically by lobe-icons' build (`npm run build:static`,
`npm run build:toc`), so no manual files are needed for those formats.

## Validation done here

Against a fresh `lobehub/lobe-icons` checkout with `src/Everruns/` added:

- `tsc --noEmit` — clean
- `eslint src/Everruns/**` — clean
- `prettier -c` — clean
- All five variants rendered and visually verified (see `preview/`).

## How to submit the PR

`lobe-icons` is an external repo, so the final fork + PR step needs your GitHub
account:

```bash
# 1. Fork lobehub/lobe-icons on GitHub, then:
git clone https://github.com/<you>/lobe-icons.git
cd lobe-icons
git checkout -b feat/add-everruns-icon

# 2. Apply this contribution (either option):
git apply /path/to/everruns.patch
#   …or copy the folder + add the export line manually:
#   cp -r /path/to/src/Everruns src/Everruns

# 3. Sanity-check, commit, push:
npm install
npm run type-check && npm run lint
git add src/Everruns src/icons.ts
git commit -m "✨ feat: add Everruns icon"
git push -u origin feat/add-everruns-icon
# open the PR against lobehub/lobe-icons:master
```

Brand source: https://everruns.com · colors navy `#0A1636`, gold `#D4A43A`.
