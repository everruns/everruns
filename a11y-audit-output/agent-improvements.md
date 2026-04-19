# Proposed Improvements — from the `a11y-audit` run

Derived from running the agent end-to-end against `dev.everruns.com` with the
prompt "Audit global chat for accessibility issues" (session
`019da1bd6745737d92a1266e84475972`, budget $7, spent $1.64).

Transcript: `session-019da1bd6745737d92a1266e84475972.jsonl` (this folder).

The run surfaced issues at two layers. This doc splits proposals accordingly:

- **Part A — `a11y-nanny` (agent repo)**: changes to the agent's own
  definition, instructions, specs, and skills.
- **Part B — `everruns` (platform repo)**: changes to the SDK, CLI,
  tools, and snapshots the agent depends on.

---

## Observed failure modes (with root-cause attribution)

| # | Symptom | Root cause lives in |
|---|---------|---------------------|
| 1 | Daytona snapshot came up empty (no app, Playwright, axe-core) | **a11y-nanny** (own the bootstrap via a skill; treat the snapshot as bare metal). Platform-side snapshot refresh kept as a latency optimisation, not a blocker. |
| 2 | First turn returned a plan, no action, until user said "Proceed" | **a11y-nanny** (SKILL Phase 1 wording) |
| 3 | Defaulted to `http://localhost:9300` instead of `dev.everruns.com` | **a11y-nanny** (AGENTS.md + target spec) |
| 4 | Gave up after 2 attempts on "execution context destroyed" | **a11y-nanny** (Resilience rule is soft) |
| 5 | axe wrote `reports/chat/axe-results.json` but agent could not read `violations` | **everruns** (tool output truncation) + **a11y-nanny** (no jq-summary step) |
| 6 | `activate_skill` invoked 3x for one skill | **everruns** (no session-level idempotency) + **a11y-nanny** (no guidance) |
| 7 | CLI `/v1/agents/import` rejected API-key principals in multi-org accounts | **everruns** (missing `X-Org-Id` header) — already fixed in `ee17a05` |

---

## Part A — Changes in `a11y-nanny`

### A1. Own the sandbox bootstrap via a dedicated skill

**New file:** `agents/a11y-audit/.agents/skills/bootstrap-sandbox/SKILL.md`
(plus touches to `agents/a11y-audit/.agent.md` to declare the skill and to
`agents/a11y-audit/specs/sandbox-setup.md` to point at it).

The `everruns-a11y-nanny` Daytona snapshot is shared infra the agent doesn't
own. Relying on it being correctly provisioned is the single biggest source
of session-time waste and uncertainty. Instead, treat the snapshot as
"bare metal" and have the agent install its own tooling on every cold
start via a dedicated skill.

The Daytona integration today exposes `daytona_create_sandbox` +
`daytona_list_snapshots` (read) but no snapshot-write tool
(`integrations/daytona/src/tools.rs`), so the agent cannot publish a shared
image. A self-bootstrapping skill side-steps that constraint entirely.

Proposed skill shape (`SKILL.md` phases):

1. **Probe** — single `daytona_exec`:
   ```bash
   command -v node >/dev/null && command -v jq >/dev/null \
     && node -e "require('@axe-core/playwright')" 2>/dev/null \
     && echo READY || echo BOOTSTRAP
   ```
2. **Bootstrap** (only on `BOOTSTRAP`):
   ```bash
   # Minimal, pinned, idempotent.
   apt-get update -qq && apt-get install -y --no-install-recommends jq curl
   curl -fsSL https://deb.nodesource.com/setup_lts.x | bash -
   apt-get install -y --no-install-recommends nodejs
   npm install -g --no-audit --no-fund \
     playwright@1 @axe-core/playwright@4
   npx playwright install --with-deps chromium
   ```
3. **Cache** — write a marker and the installed versions to
   `/.outputs/bootstrap.ok` so subsequent `daytona_exec` calls in the same
   sandbox hit the "already ready" path without re-probing.
4. **Report** — log the probe outcome, bootstrap duration, and tool
   versions in the session notes so later phases (and the final report)
   can reference them.

Pros:
- No dependency on the platform team rebuilding the snapshot.
- No dependency on a new Daytona snapshot-write tool.
- Deterministic: the agent ships with its tooling recipe, pinned.
- Works on any Daytona base image, not just `everruns-a11y-nanny`.

Cost:
- ~30–60 s on cold starts where the snapshot is bare (e.g. plain Ubuntu
  or a stale image). Zero extra cost when the snapshot already has
  everything (the probe returns `READY`).

Follow-ups to keep separate:
- Platform-side B1 (bake tooling into the snapshot) remains valid as a
  latency optimisation — the skill makes it optional, not obsolete.
- If Daytona ever exposes snapshot-commit, a second skill could commit
  the bootstrapped state back under a versioned tag.

### A2. Bias the first turn toward execution, not planning

**File:** `agents/a11y-audit/.agents/skills/audit-a11y/SKILL.md`

Replace Phase 1 "propose a plan" with:

> If the user's request names a single concrete target (URL or UI surface),
> skip the plan and proceed directly to Phase 2. Only produce a plan when
> the target is ambiguous or spans more than one surface.

### A3. Prefer the user-supplied public URL over `localhost`

**File:** `agents/a11y-audit/AGENTS.md` and `agents/a11y-audit/specs/target.md`

Change target precedence to:

1. URL explicitly named in the prompt (e.g. "dev.everruns.com").
2. URL in `EVERRUNS_TARGET_URL` env var.
3. `http://localhost:9300` **only** if neither is set and the sandbox has
   the app running locally.

Add a one-line sanity check: `curl -sSf --max-time 5 "$TARGET/healthz"` before
launching Playwright. If the target 4xx/5xx or times out, stop and report.

### A4. Enforce "minimum 3 attempts" mechanically

**File:** `agents/a11y-audit/AGENTS.md` — "Resilience" section

Turn the soft rule into a checklist the agent must tick:

```
Before declaring a subtask "failed":
- [ ] Attempt 1: original strategy
- [ ] Attempt 2: strategy varied on one axis (wait, selector, transport)
- [ ] Attempt 3: fallback transport (CDN addScriptTag, raw fetch, screenshot)
Record each attempt's exact command and error in the session notes.
```

### A5. Handle SPA → login redirects as a first-class audit path

**File:** `agents/a11y-audit/.agents/skills/audit-a11y/SKILL.md` — new phase
between 4 and 5.

> **Phase 4a — Post-redirect resolution.** After `page.goto(url)`, wait for
> `networkidle`, read `page.url()`, and audit **that** URL. If it differs
> from the requested URL, note the redirect chain in the report and audit
> the terminal page. Use `page.waitForLoadState('networkidle')` +
> `page.evaluate(() => window.stop())` before injecting axe.

Include the CDN fallback snippet for when local `@axe-core/playwright` is
unusable:

```js
await page.addScriptTag({ url: 'https://cdnjs.cloudflare.com/ajax/libs/axe-core/4.8.2/axe.min.js' });
const results = await page.evaluate(async () => await axe.run());
```

### A6. Summarise large axe output with `jq` before reading it

**File:** `agents/a11y-audit/.agents/skills/audit-a11y/SKILL.md` — Report phase

Prescribe a summarisation step inside the sandbox so the agent never tries to
read the full raw blob through a truncated tool channel:

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

Then read the summary file.

### A7. Activate each skill exactly once per session

**File:** `agents/a11y-audit/AGENTS.md`

Add a one-liner:

> Activate each skill exactly once per session. Re-use the handle rather
> than calling `activate_skill` again.

### A8. Ship a "chat surface" playbook

**File:** `agents/a11y-audit/.agents/skills/audit-a11y/SKILL.md`

Disambiguate "global chat" without a user round-trip:

| Surface           | URL pattern              | Auth needed |
|-------------------|--------------------------|-------------|
| Marketing chat    | `/`, `/chat-demo`        | none        |
| Product chat      | `/app/chat`, `/chat`     | session     |
| Login wall ahead  | `*.propelauthtest.com`   | n/a (audit as-is) |

---

## Part B — Changes in `everruns`

### B1. Ship a non-empty `everruns-a11y-nanny` Daytona snapshot *(optional, latency optimisation)*

**Where:** whoever owns the snapshot image (platform/infra).

The snapshot named in `agents/a11y-audit/specs/sandbox-setup.md` came up with
no Node, no Playwright, no axe-core, and no pre-provisioned app. A11y-nanny's
**A1 (self-bootstrap skill)** removes this as a blocker — the agent will
install its own tooling. B1 is kept as a cold-start latency optimisation
only: if the snapshot ships the following pre-installed, A1's probe short-
circuits and saves ~30–60 s per cold start:

- Node LTS + `npm`
- `playwright` + `@playwright/test` + Chromium (installed via
  `playwright install --with-deps chromium`)
- `@axe-core/playwright`
- `jq`
- Optional: a local copy of the everruns dev stack for offline audits.

Alternative / longer-term: expose a `daytona_commit_sandbox` tool from the
Daytona integration so agents can publish their own versioned snapshots
after bootstrapping, rather than depending on a human-maintained image.

### B2. Stop truncating large tool outputs silently — **filed: [EVE-339](https://linear.app/everruns/issue/EVE-339/unified-truncation-contract-for-all-reading-tools)**

Generalised during review: the same gap exists across `read_file`,
`sqldb_query`, `browserless_content`, the four sandbox `*_read_file`
variants, and several others. EVE-339 defines a unified
`ReadingToolOutput` envelope (`truncated`, `bytes_returned`,
`bytes_total`, `next_offset`, `reason`), extends `specs/tool-execution.md`
to describe it, and requires a conformance test harness.

### B3. Make `activate_skill` idempotent within a session — **filed: [EVE-337](https://linear.app/everruns/issue/EVE-337/make-activate-skill-idempotent-within-a-session)**

Three activations of the same skill in one session is wasted tokens and
latency. EVE-337 proposes tracking active skills on the session and
returning `{"already_active": true}` on re-activation.

### B4. `X-Org-Id` forwarding on CLI import — **done (`ee17a05`), tracking: [EVE-338](https://linear.app/everruns/issue/EVE-338/cli-forward-x-org-id-on-v1agentsimport-already-fixed)**

The CLI's direct `/v1/agents/import` POST did not propagate `X-Org-Id`,
so API-key principals with access to multiple orgs got
`Multiple organizations available. ...`. Fix landed on this branch.
EVE-338 tracks the merge + the follow-up audit of remaining direct
`reqwest` calls in `crates/cli/`.

### B5. SDK: first-class `X-Org-Id` support — **filed: [everruns/sdk#82](https://github.com/everruns/sdk/issues/82)**

Replace the `EVERRUNS_ORG_ID` env-var workaround with a typed
`Client::builder().org_id(...)` option and have `Everruns::from_env()`
honour the env var. Precedence: builder > env > omitted.

### B6. Surface axe / Playwright as first-class tools or a toolkit

**Where:** new `toolkits/a11y-auditkit/` or extend `bashkit`.

Every a11y audit needs roughly the same four moves (launch browser, goto,
inject axe, collect violations). Shipping a thin `run_axe(url)` tool would
let the agent skip the Playwright plumbing entirely and remove whole classes
of "execution context destroyed" failures — and would remove the need for
A5's CDN fallback on the agent side.

---

## Budget / cost attribution

- 60 messages, 1.03M input / 8.2K output / 671K cached tokens on `gpt-5.1`.
- $1.64 of $7 consumed.
- Top three cost drivers and where the fix lives:
  1. Re-reading truncated axe JSON → **B2** (platform) + **A6** (agent).
  2. Rebuilding Playwright tooling after empty snapshot → **B1** (platform) + **A1** (agent).
  3. Plan-only first turn + "Proceed" round-trip → **A2** (agent).

## Not in scope

- UI for downloading the JSONL export — CLI export is sufficient.
- Authenticated Playwright sessions against `/chat` — that needs test
  credentials and is orthogonal to these changes.
