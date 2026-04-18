# Proposed Improvements — `a11y-audit` agent

Derived from running the agent end-to-end against `dev.everruns.com` with the
prompt "Audit global chat for accessibility issues" (session
`019da1bd6745737d92a1266e84475972`, budget $7, spent $1.64).

See `session-019da1bd6745737d92a1266e84475972.jsonl` in this folder for the
raw transcript used to draw these conclusions.

## Summary of observed failure modes

1. Snapshot `everruns-a11y-nanny` came up **empty** (no pre-provisioned app,
   no Playwright, no axe-core). The agent waited until after creating the
   sandbox to detect this and had to rebuild tooling on the fly.
2. First turn returned a plan only — no execution — until the user replied
   "Proceed". The audit SKILL does not instruct the agent to act on the first
   turn when the request is unambiguous.
3. Target resolution assumed `http://localhost:9300`, but the instance was
   already reachable at `https://dev.everruns.com`. The agent only switched
   targets after the user redirected it.
4. `/chat` redirected unauthenticated traffic to
   `55972458.propelauthtest.com/en/login`. Playwright + axe fired before the
   redirect completed, producing "Execution context was destroyed". The agent
   gave up after **2** attempts despite AGENTS.md saying "minimum 3".
5. The final axe run succeeded and wrote `reports/chat/axe-results.json` in
   the sandbox, but the agent reported "violations hidden by tool-output
   truncation" instead of paginating / filtering the JSON.
6. `activate_skill` was invoked 3 times for the same skill — the workflow
   does not cache that the skill is already active for the session.

## Proposed changes

### 1. Detect empty snapshot up front, fall back deterministically

**File:** `specs/sandbox-setup.md`

Add an explicit probe step before trusting the snapshot:

```bash
# First command after create_sandbox
test -x /usr/bin/node && test -d /app && node -e "require('@axe-core/playwright')" \
  || echo "SNAPSHOT_EMPTY"
```

If `SNAPSHOT_EMPTY` is printed, branch to the documented fallback path
(install Node, Playwright, `@axe-core/playwright`, Chromium) **before** any
audit work. Today the fallback is mentioned but not gated on a probe, so the
agent tries to use missing tools and burns tokens recovering.

### 2. Bias the first turn toward execution, not planning

**File:** `.agents/skills/audit-a11y/SKILL.md`

Replace Phase 1 "propose a plan" with:

> If the user's request names a single concrete target (URL or UI surface),
> skip the plan and proceed directly to Phase 2. Only produce a plan when
> the target is ambiguous or spans more than one surface.

The observed session wasted one full round-trip waiting for "Proceed".

### 3. Prefer the user-supplied public URL over `localhost`

**File:** `AGENTS.md` and `specs/target.md`

Current default is `http://localhost:9300`. Change the precedence to:

1. URL explicitly named in the prompt (e.g. "dev.everruns.com").
2. URL in `EVERRUNS_TARGET_URL` env var.
3. `http://localhost:9300` **only** if neither is set and the sandbox has
   the app running locally.

Add a one-line sanity check: `curl -sSf --max-time 5 "$TARGET/healthz"` before
launching Playwright. If the target 4xx/5xx or times out, stop and report.

### 4. Enforce "minimum 3 attempts" mechanically

**File:** `AGENTS.md` — "Resilience" section

Today the rule is a guideline. Make it a checklist the agent must tick:

```
Before declaring a subtask "failed":
- [ ] Attempt 1: original strategy
- [ ] Attempt 2: strategy varied on one axis (wait, selector, transport)
- [ ] Attempt 3: fallback transport (CDN addScriptTag, raw fetch, screenshot)
Record each attempt's exact command and error in the session notes.
```

In the observed run the agent stopped at 2 and had to be nudged by the user.

### 5. Handle SPA → login redirects as a first-class audit path

**File:** `.agents/skills/audit-a11y/SKILL.md` — new phase between 4 and 5

> **Phase 4a — Post-redirect resolution.** After `page.goto(url)`, wait for
> `networkidle`, read `page.url()`, and audit **that** URL. If it differs
> from the requested URL, note the redirect chain in the report and audit
> the terminal page. Use `page.waitForLoadState('networkidle')` +
> `page.evaluate(() => window.stop())` before injecting axe, to avoid
> "Execution context was destroyed".

Also add the CDN fallback verbatim:

```js
await page.addScriptTag({ url: 'https://cdnjs.cloudflare.com/ajax/libs/axe-core/4.8.2/axe.min.js' });
const results = await page.evaluate(async () => await axe.run());
```

### 6. Teach the agent to read large JSON output without truncation

**File:** `.agents/skills/audit-a11y/SKILL.md` — Phase "Report"

When `axe-results.json` exceeds ~64 KB the `daytona_read_file` output gets
truncated. Prescribe a summarisation step inside the sandbox so the agent
never tries to read the full blob:

```bash
jq '{
  url: .[0].url,
  counts: (.[0].violations | group_by(.impact)
           | map({(.[0].impact // "none"): length}) | add),
  violations: [.[0].violations[] | {
    id, impact, help, helpUrl,
    nodeCount: (.nodes | length),
    sample: (.nodes[0:2] | map({target, html}))
  }]
}' reports/chat/axe-results.json > reports/chat/axe-summary.json
```

Then `daytona_read_file reports/chat/axe-summary.json` — small, structured,
paginatable.

### 7. Cache `activate_skill` for the session

**File:** `.agent.md` — capabilities

Skill activation is idempotent but the agent reinvoked it 3x in the same
session. Add a one-liner to AGENTS.md:

> Activate each skill exactly once per session. Re-use the handle rather
> than calling `activate_skill` again.

### 8. Add a concrete "chat surface" playbook

**File:** `.agents/skills/audit-a11y/SKILL.md`

The prompt "Audit global chat" is ambiguous between the marketing site,
`/chat`, and the in-product conversation pane. Ship a table mapping common
surface names to URL patterns + required auth, so the agent can pick the
right target without a round-trip:

| Surface           | URL pattern              | Auth needed |
|-------------------|--------------------------|-------------|
| Marketing chat    | `/`, `/chat-demo`        | none        |
| Product chat      | `/app/chat`, `/chat`     | session     |
| Login wall ahead  | `*.propelauthtest.com`   | n/a (audit as-is) |

## Budget / cost notes

- 60 messages, 1.03M input / 8.2K output / 671K cached tokens on `gpt-5.1`.
- $1.64 of $7 consumed. The three biggest cost items were:
  1. Re-reading axe result JSON (truncated, retried).
  2. Rebuilding Playwright tooling after the empty-snapshot surprise.
  3. Plan-only first turn + "Proceed" round-trip.

Changes 1, 2, and 6 above each directly target one of those line items.

## Out of scope for this proposal

- Rebuilding the `everruns-a11y-nanny` Daytona snapshot itself — that is a
  platform-side change, not an agent change. Flag it separately to whoever
  owns the snapshot.
- Adding a UI for downloading the JSONL export — the CLI export path is
  already sufficient for this workflow.
