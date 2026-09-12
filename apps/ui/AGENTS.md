## UI

Next.js app. Iterate without the full stack: `./node_modules/.bin/next dev --port 9120`.

### Design system (Slate)

`src/app/design-system.css` is the runtime source of truth; [`DESIGN.md`](./DESIGN.md) is its
agent-readable companion in the [DESIGN.md format](https://github.com/google-labs-code/design.md).
Change tokens in both together, then run `pnpm run design:lint`.

Design intent and brand rationale live in [`knowledge/ui/brand.md`](../../knowledge/ui/brand.md).

<!-- BEGIN:nextjs-agent-rules -->

# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->
