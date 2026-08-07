## UI

Next.js app. Iterate without the full stack: `./node_modules/.bin/next dev --port 9120`.

### Design system (Slate)

`src/app/design-system.css` is the runtime source of truth; [`DESIGN.md`](./DESIGN.md) is its
agent-readable companion in the [DESIGN.md format](https://github.com/google-labs-code/design.md).
Change tokens in both together, then run `pnpm run design:lint`.

Design intent and brand rationale live in [`knowledge/ui/brand.md`](../../knowledge/ui/brand.md).
