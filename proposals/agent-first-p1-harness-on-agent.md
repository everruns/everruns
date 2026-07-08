# P1 — Agent owns its harness (implementation plan)

Status: implementation plan for Phase 1 of `proposals/agent-first-architecture.md`.
Scope is deliberately narrow and backward-compatible: it lands the foundation
the other four phases build on, without breaking harness-first creation or
single-agent sessions.

## What P1 is

1. **Agent gains a required `harness_id`.** The author picks the runtime once,
   on the agent, defaulting to the org's `generic` harness.
2. **Session creation becomes agent-first.** `POST /v1/sessions { "agent": ... }`
   resolves the harness from the agent when no harness is supplied.
3. **The App create form prefills the harness from the chosen agent** (UI only).

## What P1 is NOT (deferred to later phases)

- No `session_participants` — `session.agent_id` stays a single pointer (P2).
- No removal of `App.harness_id` from the backend — Apps keep their harness
  field; only the *create form* prefills it (removal rides with §5 App slimming).
- No memory scopes, triggers, or identity convergence.
- The config-overlay fold is **untouched** (see §6) — harness stays the
  environment layer sourced from the harness chain, never from the agent.

## Decisions to confirm

| # | Decision | Recommendation |
|---|----------|----------------|
| D1 | How does the API name the agent on session create? | Add `agent_name: Option<String>` beside the existing `agent_id`, mirroring the existing `harness_id`/`harness_name` split. Keep both mutually exclusive. The product-facing `{ "agent": "support" }` sugar (accepts id-or-name) can be a thin alias later; the CLI already does id-vs-name detection client-side. |
| D2 | `harness_id` required & non-null on the agent? | Yes. NOT NULL column, backfilled to `generic`. `CreateAgentRequest.harness_id` required; `UpdateAgentRequest.harness_id` optional (`None` = unchanged, never clearable to NULL). |
| D3 | When a session pins an `agent_version_id`, does the harness come from the pinned version or the live agent row? | P1: from the **live agent row** at create time (matches today's "resolve harness at create"). Version-pinned harness is a refinement tracked as an open question, not P1. |
| D4 | Precedence when both an explicit harness and an agent are supplied? | Explicit harness wins (override), then `agent.harness_id`, then org default, then built-in fallback. |

## 1. Data model & migration

**Core type** — `crates/core/src/agent.rs`. Add to `Agent` (struct at
`agent.rs:187-300`), modeled on `default_model_id` (`agent.rs:220-224`) but
**required**:

```rust
// import HarnessId at agent.rs:20 alongside AgentId/ModelId
#[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
pub harness_id: HarnessId,
```

`HarnessId` already exists (`crates/core/src/typed_id.rs:617`). Because the
field is required, drop `Option`/`skip_serializing_if`. Update the two literal
constructions in this file's tests (`agent.rs:402-428`) and add the field to
the JSON fixture in `test_agent_deserialize_from_api_json` (`agent.rs:438-457`)
— a required field without a serde default breaks that deserialization test.

**Migration** — `crates/server/migrations/093_agents_harness_id.sql` (next free
number; latest is `092_app_channel_type_public_chat.sql`). Three steps,
following the per-org backfill precedent in
`058_backfill_default_marketplace.sql`:

```sql
-- 1. add nullable
ALTER TABLE agents ADD COLUMN harness_id UUID REFERENCES harnesses(id);

-- 2. backfill per org from the built-in generic harness, base as a safety net
UPDATE agents a
SET harness_id = COALESCE(
  (SELECT h.id FROM harnesses h
     WHERE h.org_id = a.org_id AND h.name = 'generic' AND h.is_built_in
     LIMIT 1),
  (SELECT h.id FROM harnesses h
     WHERE h.org_id = a.org_id AND h.name = 'base' AND h.is_built_in
     LIMIT 1))
WHERE a.harness_id IS NULL;

-- 3. enforce
ALTER TABLE agents ALTER COLUMN harness_id SET NOT NULL;
```

FK clause style mirrors `harness_capabilities.harness_id`
(`001_base_schema.sql:365`). After adding the file, run
`bash scripts/lib/check-migration-ordering.sh` (per `AGENTS.md` and
`crates/server/migrations/AGENTS.md`); do not touch merged migration bodies
(`crates/server/tests/migration_history_test.rs` locks them).

**Guarantee generic exists:** `base` and `generic` are required built-ins
provisioned for every org at init and kept current by
`reconcile_built_in_harnesses` (`org_init.rs:235`), so step 2 always resolves.
The `base` COALESCE branch is belt-and-suspenders for any pre-init org row.

**Storage rows** — `crates/server/src/storage/models.rs`: add
`harness_id: HarnessId` to `AgentRow` (near `default_model_id`, `models.rs:411`),
`CreateAgentRow` (`models.rs:520`), and (optional, `None`-means-unchanged)
`UpdateAgent` (`models.rs:542`).

**Postgres repo** — `crates/server/src/storage/repositories/agents.rs`: thread
`harness_id` through every column list: create INSERT/RETURNING (`:19,:21`),
`create_agent_with_id` (`:55,:82`), the get/list SELECTs (`:110,:144,:192,:227`),
`update_agent` (`SET harness_id = COALESCE($N, harness_id)`, `:255,:270`), and
both upsert paths (`:347,:364,:398,:414`).

**In-memory store** — `crates/server/src/storage/memory/agents.rs`: add the
field to every `AgentRow { .. }` literal (`:19,:82,:98,:351,:420`) and the
model assignments (`:27,:106,:247,:338,:359,:428`). Storage parity is enforced
by `crates/server/tests/repository_conformance_test.rs`.

## 2. Agent domain

**DTOs** — `crates/server/src/domains/agents/types.rs`:
- `CreateAgentRequest` (`:16-82`): add required `harness_id: HarnessId` /
  optional `harness_name: String` (parity with model handling at `:42-44`).
- `UpdateAgentRequest` (`:85-157`): add `harness_id: Option<HarnessId>`.

**Validation** — mirror `validate_model_id`
(`domains/agents/queries.rs:356-368`) with a `validate_harness_id` that checks
the harness exists, is in the caller's org, and is `active`; call it from
`CreateAgent` (`commands.rs:205`), `UpdateAgentCmd` (`:510`), and `UpsertAgent`
(`:705`). Resolve `harness_name`→id using the existing
`StorageBackend::get_harness_by_name` (`backend.rs:645`).

**Construction sites** (a required field is a compile break until all are
updated) — set `harness_id` in: both `CreateAgentRow` builders in `CreateAgent`
(`commands.rs:212,:241`), `UpsertAgent` (`:727`), the upsert handler literal
(`api/agents.rs:831-846`), `CopyAgent` (`:822`), `ForkAgentVersion`
(`:1455-1470`), `import_from_example` (`:1041-1056`), and `import_from_file`
(`:1128-1147`).

**Agent-as-file round trip** — `api/agents.rs`: add harness to the `AgentFile`
DTO (`:93-118`), the export writer `agent_to_markdown` (`:1195-1197`), and the
parser fallback `parse_agent_content` (`:1268-1279`). Accept a harness
*name* in the file (portable across orgs) and resolve at import.

**AgentVersion snapshots** — harness must be captured or it is lost on
rollback/fork. Add `harness_id` to `authored_config` (`queries.rs:115-131`) and
`build_resolved_config` (`commands.rs:948-969`), read it back in
`version_to_agent` (`queries.rs:157,:223`), and restore it in the rollback
(`commands.rs:1301-1322`) and fork (`:1455-1470`) request builders. **Note:**
adding a field to `authored_config` changes `config_hash`, so the next
auto-snapshot after deploy records a version bump for every edited agent —
expected and benign.

**Seed / examples** — `SeedAgent` (`seed.rs:391-405`) has no harness today and
becomes an agent only via `import_from_example`, which currently hardcodes
`default_model_id: None`. Give `import_from_example` a harness default:
resolve the org's `generic` harness id (add a `generic_harness_id(db, org_id)`
helper next to `org_init::base_harness_id`, `org_init.rs:84-94`) and pass it
when the seed doesn't specify one. Optionally add an explicit `harness` name to
`SeedAgent` for examples that need a specific runtime (e.g. coding examples).

## 3. Agent-first session creation

The whole change is at session-create time; **no worker/turn-time change** is
needed because turn assembly already loads the harness chain from
`session.harness_id` (`runtime_context.rs:157`), which we snapshot correctly at
create.

**Request DTO** — `crates/server/src/api/sessions.rs` `CreateSessionRequest`
(`:40-139`): add `agent_name: Option<String>` beside `agent_id` (`:57`)
[D1]. `harness_id`/`harness_name` (`:47,:53`) stay optional; update the doc
comment on `:43` from "org default harness" to "derived from the agent, else
org default." Confirm whether `domains/sessions/types.rs` re-exports or
redefines this DTO and keep them in sync.

**Command** — `crates/server/src/domains/sessions/commands.rs`
`CreateSession::execute` (`:69-192`). Reorder so the **agent resolves first**:
1. Keep the harness mutual-exclusion check (`:76-80`); add the same for
   `agent_id`/`agent_name`.
2. Resolve the agent (currently at `:146-160`, `get_agent_by_public_id`;
   add a by-name path) **before** harness resolution, capturing
   `agent_row.harness_id`.
3. Pass the agent's harness as a new default into harness resolution.

**Resolver** — `domains/sessions/queries.rs` `resolve_session_harness_id`
(`:31-59`): thread an `agent_harness_id: Option<Uuid>` argument and apply
precedence [D4]: explicit request → `agent_harness_id` → org
`default_harness_id` → built-in fallback. The `harness_name == "default"`
special case and org-settings path (`commands.rs:96-120`) are preserved.

Backward compatibility: a request with an explicit harness and no agent, or an
agent with no harness, both keep working exactly as today.

## 4. Config fold — unchanged (why this is safe)

`AgentConfigOverlay::from(&Agent)` (`config_layer.rs:202-216`) reads only the
agent's additive fields and must **not** be extended to read `harness_id`. The
harness overlays come from the harness chain resolved from `session.harness_id`
in `resolve_runtime_capabilities` (`runtime_context.rs:284-290`), independent of
the agent. Adding `harness_id` to `Agent` therefore has zero effect on the
folded RuntimeAgent. This is the invariant that keeps P1 a pure
ownership/addressing move rather than an execution-semantics change.

## 5. API surface & OpenAPI

- Regenerate the committed spec after the DTO changes:
  `./scripts/export-openapi.sh` → `docs/api/openapi.json`. Enforced by
  `scripts/lib/pre-pr.sh` and CI job `openapi-check`
  (`.github/workflows/ci.yml`); `openapi_coverage_test.rs` /
  `openapi_descriptions_test.rs` must stay green.
- Regenerate UI types: `cd apps/ui && pnpm api-types:generate` (reads
  `docs/api/openapi.json` → `src/lib/api/generated/openapi.ts`); verify with
  `pnpm api-types:check`.

## 6. UI (`apps/ui`)

TypeScript request/response types the UI imports are hand-written in
`src/lib/api/legacy-api-types.ts` (re-exported via `types.ts`):
- `Agent` (`:44-69`) — add `harness_id: string`.
- `CreateAgentRequest` (`:148-164`) — add required `harness_id: string`.
- `UpdateAgentRequest` (`:166-183`) — add `harness_id?: string`.
- `CreateSessionRequest` (`:3839-3861`) — stays as-is (`harness_id?`,
  `agent_id?`); add `agent_name?` if D1 lands.

**Agent forms** — reuse the existing `HarnessSelect`
(`src/components/harness/harness-select.tsx`, already used by the App form):
- Create: `agents/new/page.tsx` — add to `formData` (`:35-41`) and the create
  payload (`:85-93`); render `HarnessSelect` near `ModelPicker` (`:179`).
- Edit: `agents/[agentId]/edit/page.tsx` — add to `FormData` (`:63-70`),
  initialize from `agent.harness_id` (`:96-115`), include in the update payload
  (`:178-192`).
- Make harness required in `agentFormSchema` (`src/lib/form-validation.ts`).

**Agent-first session creation** — switch the "New session" call sites to take
the harness from the agent instead of the org default:
- `agents/[agentId]/page.tsx` `handleNewSession` (`:117-128`) — send
  `{ agent_id }` and let the server derive harness (or pass `agent.harness_id`).
- `agents/[agentId]/sessions/page.tsx` (`:53-64`).
- `sessions/sessions-page-client.tsx` New Session dialog (`:259-306`) — when an
  agent is selected, derive/hide the harness picker; keep the picker for the
  no-agent path.
- `dashboard/page.tsx` dialog (`:149-174`) — already agent-only; drop the
  org-default harness resolution (`:58-73`).
- Global chat (`chat/page.tsx` → `getOrCreateChatSession`) is server-managed —
  out of scope.

**App create form** — `apps/new/page.tsx`: on agent select (`:149`), prefill
`harnessId` from the chosen agent's `harness_id` via an effect; keep the
existing query-string prefill (`:33-34`). Backend `CreateAppRequest` is
unchanged.

## 7. CLI (`crates/cli`)

- `agents create` / `agents update` (`commands/agents.rs:35-106`): add a
  `--harness/-H` flag; plumb through `create_from_flags` (`:998-1036`) and
  `update_from_flags` (`:1040-1067`). This needs a `.harness_id(...)` /
  `.harness_name(...)` builder on the external `everruns-sdk`
  `CreateAgentRequest` (see §8). The file-based path (`--file`) forwards
  untyped JSON, so a `harness` key passes through with no CLI struct change.
- `sessions create` (`commands/sessions.rs`): already treats `--agent`
  independently of `--harness`, and `--harness` is already `Option`. When
  `--agent` is given and `--harness` omitted, `build_create_session_body`
  (`:346`) simply omits harness and the server derives it — only the `--help`
  text and `specs/cli.md` (`:55,:57,:76-80`) need updating.

## 8. SDKs (external repos)

No SDK source lives in this repo; all three are published packages regenerated
from `docs/api/openapi.json`:
- **Rust** `everruns-sdk` (pinned `Cargo.toml:151`) — add harness builder
  methods to `CreateAgentRequest`; bump the version here and in the CLI dep.
- **Python** `everruns-sdk`, **TypeScript** `@everruns/sdk` — pick up the new
  agent field on regeneration; session agent-first already works.
- Update the in-repo compat harness: `.github/scripts/sdk-compat/{test-rust,
  test-python,test-typescript}.sh` and version pins. These are separate
  upstream PRs sequenced after the spec change merges.

## 9. Docs & specs

- Specs: `specs/models.md` (Agent — add `harness_id`), `specs/concepts.md`
  (Agent no longer "may or may not" carry a harness; session-from-agent),
  `specs/harness-types.md` (harness selection moves onto the agent),
  `specs/apps.md` (create form prefill note), `specs/cli.md` (flags).
- Product docs / examples: `docs/how-to/automate-with-the-cli.md`,
  `docs/how-to/define-agents-as-files.md`, `docs/tutorials/*`,
  `examples/agents/*.md`, `examples/hackernews-reader/run.py`, the notebooks.

## 10. Testing

- Core: update `Agent` literals/fixtures in `crates/core/src/agent.rs`
  (`:402,:438`), `runtime_agent.rs`, and worker `grpc_adapters.rs`.
- Storage parity: extend `repository_conformance_test.rs` — a created agent
  round-trips `harness_id`; `session_input()` (`:43-60`) gains an
  agent-derived-harness case.
- Domain: in `api_integration_test.rs`, assert (a) creating an agent without a
  harness defaults to `generic`; (b) `POST /v1/sessions {agent_id}` with no
  harness snapshots the agent's harness into `session.harness_id`; (c) explicit
  harness overrides the agent's [D4]; (d) the org-default path
  (`:1189-1209`) still works for no-agent sessions.
- Versions: rollback and fork preserve `harness_id` (new assertions around
  `version_to_agent`).
- Migration: new migration applies clean and backfills; `migration_history_test`
  stays green.
- OpenAPI + UI type-check jobs green.

## 11. Rollout — PR breakdown

Each PR is independently reviewable and leaves the system working:

1. **PR-A `feat(agents): agent owns harness_id`** — core type, migration,
   storage (pg + memory), DTOs, validation, version snapshots, seed/import,
   OpenAPI regen. All construction sites updated so it compiles. Ships useful
   on its own: agents now carry a harness; sessions ignore it for now.
2. **PR-B `feat(sessions): resolve harness from agent`** — reorder
   `CreateSession`, resolver precedence, `agent_name` DTO, OpenAPI regen.
3. **PR-C `feat(ui): harness on agent forms`** — types + create/edit forms.
4. **PR-D `feat(ui): agent-first session creation + app-form prefill`**.
5. **PR-E `feat(cli): --harness on agents create/update`** (after the SDK bump).
6. **PR-F `docs/chore`** — specs, docs, examples.
7. **External** — Rust/Python/TS SDK PRs + compat-harness update.

Sequencing: A → B are the backend spine and must land first (B depends on A's
`agent.harness_id`). C/D depend on B's API shape landing in `openapi.json`. E
depends on the external Rust SDK release. F can trail.

## 12. Risks

- **Compile-break blast radius.** A required field breaks every `Agent` /
  `CreateAgentRequest` literal in tests and adapters — grep and fix in PR-A;
  consider a test-only builder to reduce churn.
- **NOT NULL backfill.** Guarded by the `generic`→`base` COALESCE; verify no
  org has zero built-in harnesses before `SET NOT NULL`.
- **config_hash churn** on first post-deploy edit (expected; documented above).
- **OpenAPI drift** fails CI if `docs/api/openapi.json` isn't regenerated —
  part of PR-A/PR-B checklists.
- **DEV_MODE** skips migrations (in-memory); the memory store must default
  `harness_id` and org-init must provision `generic` (it does) so dev sessions
  keep working.

## 13. Open questions (not blocking P1)

- Version-pinned harness [D3]: should a session bound to a pinned
  `agent_version_id` take its harness from that version's `resolved_config`
  rather than the live agent row? Deferred; P1 uses the live row.
- Should `App.harness_id` become derived/removed now or with §5 App slimming?
  P1 keeps it and only prefills in the UI.
- Do we want the `{ "agent": "<id-or-name>" }` single-field sugar in the public
  API, or is the `agent_id`/`agent_name` split enough? [D1]
