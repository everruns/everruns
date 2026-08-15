# Deep review: the library-first refactor (as of 2026-08-15)

Status: investigation. Snapshot review of the ~50 commits landed since 2026-08-01
(agentyk-inspired restructure: neutral core kernel, engine/host split, engine-owned
sessions, unified execution, multi-head workspaces), audited across architecture,
code quality, security, examples/docs, and knowledge freshness at HEAD `fe1176a`.

## Verdict

The refactor achieved the **API goal** and built unusually good enforcement around
it. It has not yet achieved the **dependency-graph goal**: every `everruns` library
user still compiles the control-plane crate, including an HTTP/TLS stack. Residual
debt is concentrated and known; nothing found is architecturally unsound.

What holds up:

- `everruns-core` depends only on `provider` + `capability`; direction is guarded by
  three tests plus `scripts/lib/check-core-public-api.sh` (pinned public-API
  snapshot, re-export bans, missing-docs ratchet). The freeze is mechanical, not
  aspirational.
- `everruns-engine` is a genuine sans-IO planner: `TurnExecution` returns
  `TurnPlan` + ordered effects; deterministic table tests, serialize→plan round
  trips, and cross-driver conformance tests (`crates/worker/tests/execution_conformance.rs`).
- The facade is value-first and agentyk-shaped: `Agent::builder()` →
  `InMemoryEngine::create` → `send_and_wait`, no `everruns_platform::` symbol
  anywhere in `crates/everruns/src`, offline-by-default at runtime.
- Async hygiene is strong (every new channel bounded with an explicit policy),
  `cargo check --workspace --all-targets` is warning-free, facade doctests pass,
  the `scale` revert (b93a36a) is byte-clean, and the migration left essentially no
  deprecated shims or TODO residue.
- Security controls did not move and did not regress: sanitizer, system allowlist,
  egress, network access, workspace policy all unchanged; org scoping on
  sessions/workspaces verified; `cff2626` credentialless resolution is fail-closed;
  the engine kernel keeps `OutputHardLimitHook` on both `ActAtom` constructors.
- `docs/how-to/migrate-to-0-18.md` and the framework getting-started docs are
  accurate against source, snippet by snippet.

## P1 — gaps against the original ask

### 1. The facade still compiles the control plane

`crates/host/Cargo.toml` depends on `everruns-platform` unconditionally, and
`crates/everruns` depends on host, so every library build compiles organizations,
payments, connectors, audit, email, evals. Host's actual platform usage is narrow:
typed context extensions (`crates/host/src/host.rs:690-710`), two capability
registrations (`crates/host/src/capabilities.rs:44-53`), one constant, one
`user_hooks` helper. The EVE-897 log entry names the fix not taken — a neutral
contracts crate below builtins/integrations/platform. That crate is the missing
piece of the original ask; the one-sided guard (`crates/core/tests/no_platform_dependency.rs`
constrains core, nothing constrains host) makes the asymmetry durable.

Worse, the "no Reqwest edge" claim in `crates/everruns/Cargo.toml:82-84` is false:
`crates/platform/Cargo.toml:99` pulls `reqwest`/`rustls` non-optionally for the
Resend email client (`crates/platform/src/email/resend.rs`, ungated in `lib.rs:117`).
Verified via `cargo tree -p everruns`: the default facade tree contains reqwest,
rustls, hyper. Cheap fix independent of the contracts crate: gate the email module
or move it to server.

Related: the facade's `local` feature activates platform's `a2a` feature
(`crates/local/Cargo.toml:32`), compiling the outbound A2A client stack whether or
not delegation is used; and `everruns-test-support` (llmsim) is a normal dependency
of the published facade (`crates/everruns/Cargo.toml:99`) — deliberate, but a
supply-chain-relevant fact.

### 2. The durable worker duplicates the engine's overlay rule

`TurnExecution::apply_plan` (`crates/engine/src/machine.rs:62-75`) owns the rule
"on `ScheduleAct`, overlay `previous_response_id`/`iteration`/`request_id` onto
`resume_state`" — but the worker never persists `execution.checkpoint()`. Instead
`crates/worker/src/unified_worker.rs:1524-1533` splices those fields into the
serialized `ActInput` JSON, and `:1013-1064` hand-extracts and re-applies them on
completion, plus a fallback that reconstructs `TurnState` from `ActInput`.
`DurableExecution` is created per boundary, advanced once, and dropped. This is the
exact in-process/durable duplication 0838d26/fc89ca8 set out to remove; if
`apply_plan` changes, the durable path silently diverges. The conformance tests
mask it because they drive `DurableExecution` directly, not the worker's JSON
round trip. Fix: persist the checkpoint, delete the hand-rolled envelope, and add a
conformance case through the wire path.

### 3. Backend residue still in the frozen kernel

- `crates/core/src/budget.rs:167-235` — `Budget`/`LedgerEntry` persistence records
  whose only non-core consumers are server CRUD domains. By the refactor's own
  "who consumes it during a turn" rule only `BudgetCheckResult`/`BudgetAction`
  belong in core; this mirrors `PaymentAccount`, which *was* moved (EVE-838). The
  log never mentions budget — a missed family, not a decision.
- `crates/core/src/session_schedule.rs` — the store contract staying is documented
  (EVE-880), but per-org quota policy read from env vars in core (`:33`) and
  denormalized ownership columns (`:170-210`) are backend concerns riding along.
- `crates/core/src/organization.rs:21-24` — `DEFAULT_ORG_ID: i64 = 1` and
  internal-row id plumbing.

The freeze also locked a very wide surface: ~90 public root modules, 1,561 pinned
API rows, a 1,152-warning missing-docs budget. Slimming is now a breaking event by
construction — an accepted trade-off, but each of the items above got frozen with it.

## P2 — quality, docs, and hardening

### 4. `reason.rs` moved into the engine un-decomposed

`crates/engine/src/execution/reason.rs` is 4,656 lines; `execute_llm_call` spans
~2,240 (`:1455-3693`) with ~15 mutable locals coordinating retry, stall watchdog,
keepalives, delta batching, guardrails, compaction, and tool assembly. The
fe1176a retry-stall bug is the direct cost: correctness depended on setting a
loop-local flag in the right match arms. The fix is correct and tested, but
`stream_has_final_output` remains a manually maintained flag; it deserves the same
single-predicate treatment as `stream_event_advances_stall_deadline` (`:275`).
Also note: retry after emitted thinking deltas re-streams reasoning to live
observers — accepted but undocumented. Same pattern smaller in `act.rs`
(`execute_single_tool`, ~845 lines).

### 5. Broken public examples

- `examples/client_side_tools.sh:36-58` — agent name fails validation
  (`crates/platform/src/agent.rs:382`), and `client_tools`/`"type": "client"`
  should be `tools`/`"type": "client_side"` (`crates/server/src/domains/agents/types.rs:70-74`);
  unknown fields are silently ignored, so step 4 hangs. Downstream steps are correct.
- `examples/agent_api_example.ipynb` — pre-refactor event model throughout:
  `event["event_type"]` vs wire field `type` (`crates/core/src/events.rs:320`),
  nonexistent `message.agent`/`session.completed`/`step.*` vocabulary, dead
  swagger-ui link. Every run fails or hangs.
- `examples/hook-bundles/README.md:23-37` — string-form `capabilities` entries are
  invalid for the JSON API (`Vec<CapabilityRef>`, `crates/capability/src/reference.rs:32`),
  plus name-validation failure. The six bundle JSONs themselves are accurate.
- `docs/framework/examples.md` and `crates/everruns/examples/README.md` omit
  `engine_sessions.rs` (both) and `workspace_heads.rs` (docs) — the two
  refactor-defining examples are invisible in the maintained learning path, and
  `scripts/lib/check-everruns-examples.sh` doesn't check listings.
- Base-URL convention split (`9301/v1` vs `9300/api/v1`) across shell/Python
  examples; with a stock server (`DEFAULT_API_PREFIX=/api`) the `/v1` forms 404.

### 6. Knowledge rot is localized but includes the biggest decisions

- `knowledge/log.md` stops at 2026-08-13. The kernel freeze (0b36338), engine-owned
  kernel/sessions (a80f12b/cb47296/6ff760f), unified execution (0838d26/fc89ca8),
  multi-head workspaces (fd97abd), and the distributed-engine add/revert
  (9c60afa/b93a36a) have no entries — and the revert deleted the feature's own log
  entry, so no rationale for either direction survives anywhere.
- `knowledge/foundations/architecture.md` — crate graph omits platform/capability/
  builtins and host's real dependency set; `sqldb_store` described as optional/
  pre-gRPC (worker code says otherwise, `crates/worker/src/worker_adapters.rs:257`);
  "Docker Compose in `local/`" contradicts the no-Docker contract; "threads and
  runs" terminology in the abstract.
- `knowledge/foundations/embedding.md:37` — says `HostComposition` lives in core
  (it's `crates/host/src/composition.rs:54`; line 13 of the same file is correct);
  field list claims a vector store that doesn't exist.
- `knowledge/foundations/code-organization.md:147-151` — describes the pre-EVE-897
  `session_sqldb` state; core no longer references sqldb at all.
- `knowledge/framework/library-experience.md:50` and `application-api.md:217` —
  stale 0.17.x-runtime compatibility posture; the compatibility crate is deleted.
- Stray: `crates/everruns/Cargo.toml:9-11` references a `runtime` alias module that
  no longer exists; `knowledge/ui/signup-experience-redesign-brief.md:43` still
  names the retired Dashboard step.

OKF conformance itself passes (`check_okf`: 143 concepts, conformant).

### 7. Security hardening (all pre-existing, none regressions)

- ce3a8b1 fixed `release.yml` completely, but the same injection class remains in
  `.github/workflows/cli-binaries.yml:63,79,88,112,166` and
  `docker-publish.yml:89-100,239-247` — `inputs.tag`/`github.ref_name` interpolated
  into `run:` bodies of jobs holding `contents: write`/GHCR push. Tag names can
  carry `$()`. Attacker must already have write access, so severity low; same
  env-indirection + shape-gate treatment as release.yml applies.
- Actions pinned by mutable tags, including `dopplerhq/cli-action@v3` in
  DOPPLER_TOKEN-bearing jobs. SHA-pinning is the standard fix.
- Process note: 9c60afa (4k lines) landed on main without a PR.
- Awareness notes: `WorkspacePolicy` is embedder-only — the hosted product's tenant
  isolation is session-sandbox/org-store scoping, not this; and
  `DirectEgressService::new()`/`McpClient::direct()` skip the system allowlist
  (currently CLI/tests only — watch future call sites).

## P3 — small cleanups

- `crates/host/src/lib.rs:83` — `#[allow(deprecated)]` over nothing deprecated.
- `AgentCapabilityConfig` → `CapabilityRef` rename unfinished: five `use … as`
  aliases across engine/facade/host/server; engine internals still use the old name.
- `crates/engine/src/lib.rs:39-62` — the ~40-module `pub(crate)` alias block from
  code motion hides symbol ownership inside engine; a mechanical import rewrite
  removes it.
- `crates/everruns/src/events.rs:198-250` — public accessors `.expect()` on
  envelope shape; make total or pin with a serde-shape test.
- `crates/host/src/host.rs:782-793` — lifecycle-effect emission failures are
  `warn!`-and-continue; a failed `turn.completed` append leaves the state machine
  advanced with the canonical log missing its terminal event. Deserves an explicit
  decision comment or propagation for the durable case.
- `phase_effects.rs` (single-variant enum) and the twin `InProcessExecution`/
  `DurableExecution` delegating newtypes are ceremony until a driver diverges —
  fine to keep as named seams, worth revisiting with finding 2.
- `crates/engine/src/execution/tool_scheduler.rs:119-122` — comment references a
  removed `debug_assert_eq!`.
- Only `everruns-capability` denies `missing_docs`; the facade/engine/host surfaces
  are documented in practice but unenforced.

## Suggested sequencing

1. Neutral contracts crate below platform (or feature-gate) + gate the Resend
   email module — closes the original ask's dependency-graph gap.
2. Durable worker checkpoint persistence + wire-path conformance test.
3. Fix the three broken examples and the two missing catalog entries.
4. Backfill `knowledge/log.md` 08-14/15 and patch the four stale foundations/
   framework docs.
5. Move budget records (and the schedule quota policy) out of core at the next
   intentional kernel break; harden the two remaining workflows.
6. Decompose `reason.rs` opportunistically, starting with a
   `stream_event_is_final_output` predicate.
