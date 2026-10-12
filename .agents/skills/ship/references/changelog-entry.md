# Changelog entry

The public changelog at <https://docs.everruns.com/changelog/> gets one entry per **completed**
user-facing feature, notable improvement, user-visible fix, breaking change, deprecation, or
significant Cloud incident, across every product (Cloud, self-hosted Platform, Framework, SDKs,
CLI, Yolop, Mira, Bashkit). Intent, taxonomy and rules:
[`knowledge/docs/changelog.md`](../../../../knowledge/docs/changelog.md).

## Does this PR owe an entry?

- It finishes something a user of the UI, API, CLI or crates will notice: **yes**.
- It is one step of a larger piece of work: **no**; the PR body's `Changelog:` line names the
  program or Linear issue so the PR that finishes it knows it owes the entry.
- Refactor, CI, dependencies, tests, internal docs, design docs: **no** (`not user-facing`).
- It changes or removes behavior an earlier entry describes: update that entry
  (`grep -ril "<feature name or slug>" docs/changelog/`): append `## Update YYYY-MM-DD`, or set
  `supersededBy: <folder>` when replaced. Fix moved links in place.

Every PR body carries one line: `Changelog: <entry path> | part of <program> | not user-facing`.

## Writing it

Create `docs/changelog/<YYYY-MM-DD>-<slug>/index.md` (merge date, 3 to 6 word slug). Copy the
frontmatter from a recent entry. Leave `release:` and `cloud:` out: `/prepare-release` and
`/release-to-saas` stamp them.

- `title` 30 to 70 characters: the change in plain words, with the product noun people search for.
- `description` 120 to 160 characters: one sentence adding detail beyond the title.
- Body 40 to 150 words: what changed, how to use it (UI path, request, CLI command or a short code
  block that compiles), then links. For a change that matters to both audiences, two paragraphs:
  **In the UI:** and **In code:**.
- `links.docs` must name pages that exist; `links.prs` uses `everruns/<repo>#<n>`. Never link
  private `saas` PRs or files.
- No marketing words, exclamation marks, future promises or em-dashes.

## Screenshots and video

A UI change shows its UI. Capture against a local stack (root `AGENTS.md`, "Local dev") with
realistic data, light theme, cropped to what changed:

```bash
cd apps/docs
node scripts/capture-changelog-media.mjs shot http://localhost:27100/<route> \
  ../../docs/changelog/<folder>/<name>@2x.png --selector "<css of the changed area>"
node scripts/capture-changelog-media.mjs video steps.json \
  ../../docs/changelog/<folder>/<name>@2x.mp4      # also writes <name>-poster@2x.png
```

Embed with `![What the screenshot shows](./<name>@2x.png)` and
`<video src="./<name>@2x.mp4" poster="./<name>-poster@2x.png" controls muted playsinline loop></video>`.
Every file sits in the entry folder and is referenced from `index.md`.

## Integration announcements

An entry that adds a new integration (a vendor, protocol host or platform Everruns or Yolop now
works with) opens with an integration card: the host product's mark, a plus, and the partner's
logo, over the entry title.

```bash
cd apps/docs
node scripts/generate-integration-card.mjs --host everruns --partner slack --name Slack \
  --title "<entry title>" --date <YYYY-MM-DD> \
  --output ../../docs/changelog/<folder>/integration@2x.png
```

`--host` is where the integration lands (`everruns`, `yolop`, `bashkit`, `mira`). `--partner` is
a logo slug in `apps/docs/src/assets/changelog/logos/` and repeats for up to three partners; a slug
without a logo file renders as its `--name` in a wordmark tile. Embed it first, with alt text that
names both sides. A UI screenshot can follow it.

## Check

```bash
cd apps/docs && pnpm run check:changelog && pnpm run build
```

The build fails on a broken docs link, a missing field, a length or word-count violation, or media
that is not a 2x PNG, MP4 with a poster, or SVG.

## Entries for other repositories

Entries for `saas`, `yolop`, `sdk`, `mira` and `bashkit` also live in `everruns/everruns`. Open a
small companion PR there with the entry, right after the product PR merges. Cloud-only `saas`
features are the exception: put the entry draft in the saas PR body under "Changelog entry", and
`/release-to-saas` publishes it once the feature is live.
