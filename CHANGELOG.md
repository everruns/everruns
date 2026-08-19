# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.20.0] - 2026-08-19

### Highlights

- **Register capabilities on a running runtime** - A host holding `Arc<InProcessRuntime>` can now make a capability discovered after composition resolvable without rebuilding the runtime, via `register_capability`/`is_capability_registered`. Registration stays separate from activation, and readers always see a consistent registry snapshot ([#3224](https://github.com/everruns/everruns/pull/3224)).
- **A2A delegation is now opt-in** - Outbound A2A delegation moved behind a new `a2a` Cargo feature, off by default, so the standard build no longer drags a second HTTP/TLS stack into consumers that only run the local runtime. Hosts that delegate to remote A2A agents enable the `a2a` feature explicitly ([#3221](https://github.com/everruns/everruns/pull/3221)).

### What's Changed

- feat(host): register capabilities on a running runtime ([#3224](https://github.com/everruns/everruns/pull/3224)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): let session deletion pass the append-only guards ([#3222](https://github.com/everruns/everruns/pull/3222)) by [@chaliy](https://github.com/chaliy)
- feat(everruns): gate a2a delegation behind an opt-in feature ([#3221](https://github.com/everruns/everruns/pull/3221)) by [@chaliy](https://github.com/chaliy)
- ci(security): run the advisory gate on a schedule ([#3223](https://github.com/everruns/everruns/pull/3223)) by [@chaliy](https://github.com/chaliy)
- ci(security): advisory-scan non-workspace lockfiles ([#3213](https://github.com/everruns/everruns/pull/3213)) by [@chaliy](https://github.com/chaliy)
- ci(release): auto-tag and publish library crates on merge ([#3212](https://github.com/everruns/everruns/pull/3212)) by [@chaliy](https://github.com/chaliy)
- fix(release): close the crate-release cone after the host 0.19 breaking bump ([#3214](https://github.com/everruns/everruns/pull/3214)) by [@chaliy](https://github.com/chaliy)
- chore(release): release library crates for the 0.19.0 cycle ([#3211](https://github.com/everruns/everruns/pull/3211)) by [@chaliy](https://github.com/chaliy)
- chore(release): make library-crate release audit a mandatory release step ([#3210](https://github.com/everruns/everruns/pull/3210)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump react, react-dom and @types/react in /apps/ui ([#3219](https://github.com/everruns/everruns/pull/3219)) by [@dependabot](https://github.com/dependabot)
- chore(deps-dev): bump @typescript/native-preview to 7.0.0-dev.20260707.2 in /apps/ui ([#3220](https://github.com/everruns/everruns/pull/3220)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump @tanstack/react-query from 5.101.2 to 5.101.4 in /apps/ui ([#3218](https://github.com/everruns/everruns/pull/3218)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump @astrojs/starlight from 0.41.3 to 0.41.7 in /apps/docs ([#3217](https://github.com/everruns/everruns/pull/3217)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump shiki from 4.3.1 to 4.4.3 in /apps/docs ([#3216](https://github.com/everruns/everruns/pull/3216)) by [@dependabot](https://github.com/dependabot)

### Crate Releases

Independently versioned crates published this cycle (smallest compatible bump, breaking classification from the public-API diff):

- `everruns-host` 0.19.0 → 0.20.1 (breaking: `capability_registry()` now returns `Arc<CapabilityRegistry>` and `capability_registry_mut()` was removed; adds `register_capability`/`is_capability_registered`. Published as 0.20.1: the 0.20.0 crate publish was blocked by a dev-dependency publish-order deadlock, fixed in the same cycle, and 0.20.1 is the identical code republished under a fresh tag.)
- `everruns` 0.18.1 → 0.18.2 (additive: new opt-in `a2a` feature; host dependency baseline cascade)
- `everruns-platform` 0.18.1 → 0.18.2 (host dependency baseline cascade)
- `everruns-test-support` 0.18.1 → 0.18.2 (host dependency baseline cascade)
- `everruns-llmsim` 0.18.1 → 0.18.2 (host dependency baseline cascade)

No crates were deleted or absorbed this cycle. `everruns-mcp` and the remaining published crates had no public-contract change and are not re-released.

### Migration Notes

- Embedders that use outbound A2A delegation must enable the `everruns` crate's new `a2a` feature; it is off by default and the standard build no longer includes the A2A client.
- Consumers of `everruns-host` that called `HostComposition::capability_registry()` now receive an `Arc<CapabilityRegistry>` snapshot instead of a `&CapabilityRegistry`, and `capability_registry_mut()` is gone — register capabilities through `register_capability`/`register_capability_overriding` instead.

## [0.19.0] - 2026-08-18

### Highlights

- **First-class sandbox and checkpoint records** - Sandboxes and checkpoints are now first-class records, giving durable runs an explicit, addressable execution substrate ([#3197](https://github.com/everruns/everruns/pull/3197)).

### What's Changed

- docs(knowledge): align current architecture decisions ([#3208](https://github.com/everruns/everruns/pull/3208)) by [@chaliy](https://github.com/chaliy)
- refactor(drivers): group provider crates ([#3207](https://github.com/everruns/everruns/pull/3207)) by [@chaliy](https://github.com/chaliy)
- fix(ci): quote container sandbox test filter ([#3206](https://github.com/everruns/everruns/pull/3206)) by [@chaliy](https://github.com/chaliy)
- refactor(architecture): consolidate runtime implementation crates ([#3205](https://github.com/everruns/everruns/pull/3205)) by [@chaliy](https://github.com/chaliy)
- refactor(server): absorb session SQLite backend ([#3204](https://github.com/everruns/everruns/pull/3204)) by [@chaliy](https://github.com/chaliy)
- refactor(host): absorb session services ([#3203](https://github.com/everruns/everruns/pull/3203)) by [@chaliy](https://github.com/chaliy)
- refactor(builtins): absorb UI prompt catalogs ([#3202](https://github.com/everruns/everruns/pull/3202)) by [@chaliy](https://github.com/chaliy)
- refactor(host): absorb direct HTTP egress ([#3201](https://github.com/everruns/everruns/pull/3201)) by [@chaliy](https://github.com/chaliy)
- build(release): version crates independently ([#3200](https://github.com/everruns/everruns/pull/3200)) by [@chaliy](https://github.com/chaliy)
- refactor(host): invert platform extension dependency ([#3199](https://github.com/everruns/everruns/pull/3199)) by [@chaliy](https://github.com/chaliy)
- test(release): catch stale entries in the publish-crates version map ([#3198](https://github.com/everruns/everruns/pull/3198)) by [@chaliy](https://github.com/chaliy)
- feat(sandbox): add first-class sandbox and checkpoint records ([#3197](https://github.com/everruns/everruns/pull/3197)) by [@chaliy](https://github.com/chaliy)
- chore(security): drop the stale quick-xml advisory ignores ([#3196](https://github.com/everruns/everruns/pull/3196)) by [@chaliy](https://github.com/chaliy)
- chore(knowledge): record the coding CLI MCP trust boundary as TM-TOOL-031 ([#3195](https://github.com/everruns/everruns/pull/3195)) by [@chaliy](https://github.com/chaliy)
- chore(docs): remove em-dashes and AI-tell wording from prose ([#3191](https://github.com/everruns/everruns/pull/3191)) by [@chaliy](https://github.com/chaliy)
- fix(release): drop stale publish-map entries (duckduckgo/platform, builtins/local) ([#3194](https://github.com/everruns/everruns/pull/3194)) by [@chaliy](https://github.com/chaliy)
- fix(release): move openui/a2ui publish pins from core to builtins ([#3193](https://github.com/everruns/everruns/pull/3193)) by [@chaliy](https://github.com/chaliy)

### Crate Releases

Independently versioned crates published this cycle (smallest compatible bump, verified with `cargo-semver-checks`):

- `everruns-host` 0.18.0 → 0.19.0 (breaking: removed `PlatformStoreFactory` and other public items during the consolidation)
- `everruns-mcp` 0.18.0 → 0.19.0 (breaking: removed a public inherent method)
- `everruns` 0.18.0 → 0.18.1 (additive: absorbed `everruns-local` as `pub mod local`)
- `everruns-builtins` 0.18.0 → 0.18.1 (additive: absorbed the UI prompt catalogs)
- `everruns-platform` 0.18.0 → 0.18.1 (additive: sandbox/checkpoint records and `container-sandbox` feature)

Retired (absorbed — consumers migrate):

- `everruns-observability` → `everruns-host::observability`
- `everruns-session-services`, `everruns-http` → `everruns-host`
- `everruns-local` → `everruns` (`pub mod local`)
- `everruns-openui`, `everruns-a2ui` → `everruns-builtins`

## [0.18.0] - 2026-08-16

### Highlights

- **The 0.18 neutral-kernel boundary** - `everruns-core` is now a neutral execution kernel. Hosted platform records — sessions, agents and agent versions, harnesses, evals and observers, connectors, system email, feature flags, and OAuth/credential infrastructure — moved out to `everruns-platform` (and sibling crates), and the logic-free `everruns-runtime` compatibility crate was removed. REST/gRPC shapes and stored schema are unchanged; see the [0.18 migration guide](https://github.com/everruns/everruns/pull/3148) and the **Breaking** notes below ([#3160](https://github.com/everruns/everruns/pull/3160), [#3101](https://github.com/everruns/everruns/pull/3101)).
- **Engine-owned sessions and unified execution** - Engines now own the session lifecycle, and a single execution path serves both in-process and durable runs ([#3179](https://github.com/everruns/everruns/pull/3179), [#3173](https://github.com/everruns/everruns/pull/3173), [#3167](https://github.com/everruns/everruns/pull/3167)).
- **Multi-head workspaces** - Framework workspaces now support multiple heads ([#3166](https://github.com/everruns/everruns/pull/3166)).
- **Metered embedding spend** - Retrieval and knowledge-index embedding spend is now metered and ledgered against the organization ([#3149](https://github.com/everruns/everruns/pull/3149), [#3163](https://github.com/everruns/everruns/pull/3163)).
- **Chats as core functionality** - Chats is always present in navigation and search for every organization with no feature opt-in, and now supports chat pinning.

### Breaking

- **Moved concrete in-memory backends out of `everruns-core`.** Embedded-host
  agent, harness, session, and provider stores now live in `everruns-host`.
  Writable deterministic `InMemoryMessageRetriever` and
  `InMemoryEventEmitter` fixtures now live in `everruns-test-support`.
  Hosted conversation history has no writable message-store replacement:
  append through `EventLog` / `HostEventEmitter` and read through
  `EventHistory`, preserving canonical-events-only writes.

- **Moved connector, OAuth, and system-email infrastructure out of
  `everruns-core`.** The hosted connector catalog — `Connector`,
  `ConnectorRegistry`, `ConnectorRegistryBuilder`, `ConnectorPlugin`,
  `ConnectorType`, `ConnectorValidation`, `ConnectorFormSchema` — and the
  system email contract with its concrete senders — `EmailSender`,
  `EmailMessage`, the templates, `NoopEmailSender`/`DisabledEmailSender`,
  `SystemEmailConfig`, `ResendEmailSender` — now live in `everruns-platform`;
  the `everruns_core::connector` and `everruns_core::email` modules are gone.
  The OAuth 2.1 protocol client (`everruns_core::oauth`: `OAuthClient`,
  `TokenSet`, `PkcePair`, discovery/registration metadata types) moved to
  `everruns_mcp::oauth::protocol`, its only consumer. `PlatformDefinition`
  no longer carries `connectors`/`email_sender`; compose them on
  `ServerAppBuilder` (`connector_registry`, `email_sender`, defaulting to the
  OSS inventory preset and the env-configured sender). The
  `CredentialProvider`/`EnvCredentialProvider`/`ProviderCredentials` seam
  moved to `everruns-provider` and is re-exported by core at its previous
  paths. Secret-bearing types (`TokenSet`, `PkcePair`, `ProviderCredentials`,
  `ResendEmailConfig`) now redact credentials in `Debug` output. Core no
  longer depends on `serde_urlencoded` or `eventsource-stream`. REST/gRPC
  shapes and stored schema are unchanged.

- **Moved the eval, observer, and feature-management records out of
  `everruns-core`.** The persisted eval aggregates (`Eval`, `EvalCase`,
  `EvalRun`, `EvalCaseResult`, `EvalRunDataset`, `EvalTarget`, `Scorer`,
  `Score`, and their lifecycle enums), the observer records (`Observer`,
  `TraceScore`, `ObserverMatch`, `LlmJudgeConfig`, scorer configs and
  lifecycle enums), and the org/product feature-flag management surface
  (`FeatureFlags`, `FeatureFlagMap`, `FeatureFlagDefinition`,
  `API_FEATURE_FLAG_DEFINITIONS`, org opt-in resolution) now live in
  `everruns-platform`; the `everruns_core::eval`, `everruns_core::observer`,
  and `everruns_core::feature_flags` modules are gone. Core keeps only the
  resolved execution feature decisions in
  `everruns_core::execution_features` (`InternalFeatureFlags` plus the new
  `ExecutionFeatureDecisions` snapshot consulted at capability-registration
  time); per-org effective decisions are resolved by the server and applied
  as an already-filtered capability list before worker execution.
  REST/OpenAPI shapes and stored schema are unchanged. (EVE-878)

- **Moved the persisted `Session` aggregate and product lifecycle enums out of
  `everruns-core`.** The database/API record — `Session`, `SessionStatus`,
  `SessionSource`, `SessionActivity`, `SessionParticipant`,
  `SessionParticipantKind`, `SessionParticipantRole` — now lives in
  `everruns-platform`. Core keeps only the portable
  `everruns_core::ExecutionSession` (session correlation values plus the
  per-session configuration overlay a turn consumes) and the neutral
  `everruns_core::SessionExecutionState` the host lifecycle drives;
  `SessionStore`/`SessionMutator` implementations now return the execution
  view, and status mutation acknowledges without exposing a stored record.
  The stored record projects into the execution view via
  `everruns_platform::Session::execution_session()` at the platform loading
  seam (server repositories, worker adapters); `SessionId`, turn/message
  correlation IDs, and portable event/message values stay in core. Framework
  hosts seed `ExecutionSession` values (`SessionBuilder` now builds one, and
  no longer takes owner/timestamp fields). Direct core consumers that touch
  the stored record should depend on `everruns-platform` and import it from
  there. REST/gRPC shapes and stored schema are unchanged.

- **Moved the stored `Harness` record and built-in provisioning templates out
  of `everruns-core`.** The persistence record — `Harness`, `HarnessStatus`,
  `merge_harness`, `merge_harness_chain` — and the provisioning templates —
  `BuiltInHarnessDefinition`, `BuiltInHarnessRole`,
  `BuiltInCapabilityDefinition` — now live in `everruns-platform`; the
  `everruns_core::harness` module is gone and `PlatformDefinition` no longer
  carries built-in harness templates (compose them on `ServerAppBuilder`
  instead). Core keeps only the portable `everruns_core::HarnessDefinition`
  (the effective, inheritance-resolved environment configuration), which
  `HarnessStore` implementations now return: parent-chain loading,
  cycle/error handling, and archived/deleted validation happen at that
  loading seam, before host execution, so the host never requests or
  receives a stored Harness. The Framework host seeds `HarnessDefinition`
  values under an embedder-chosen id (`everruns_host::SeededHarness`).
  REST/gRPC shapes and stored schema are unchanged.

- **Moved the stored `Agent` and `AgentVersion` records out of
  `everruns-core`.** The persistence records — `Agent`, `AgentVersion`,
  `AgentStatus`, `AgentVersionChangeKind`, `MAX_ADDRESSABLE_NAME_LEN`,
  `validate_addressable_name`, `generate_agent_public_id`, and
  `validate_agent_public_id` — now live in `everruns-platform`; the
  `everruns_core::agent` module is gone. Core keeps only the portable
  `everruns_core::AgentDefinition` (the authored execution configuration),
  which `AgentStore` implementations now return, and archived or deleted
  agents fail at that loading seam instead of inside snapshot projection.
  Direct core consumers that touch the stored records should depend on
  `everruns-platform` and import them from there; execution-side code should
  consume `AgentDefinition` or the resolved execution snapshot. REST/gRPC
  shapes and stored schema are unchanged.

- **Removed the `everruns-runtime` crate.** It was a logic-free 0.17.x
  compatibility layer over `everruns-host` and is no longer published. The
  deprecated `everruns::runtime` module alias is removed with it.

  Migrate as follows:

  - **Ordinary applications** depend on `everruns` and use `Agent`, `Model`,
    and `Session`:

    ```bash
    cargo remove everruns-runtime
    cargo add everruns
    ```

    ```rust
    use everruns::{Agent, Model};
    ```

  - **Custom execution hosts** depend on `everruns` plus `everruns-host` and
    only the focused siblings they need (`everruns-engine`, `everruns-mcp`,
    `everruns-local`, provider and integration crates):

    ```bash
    cargo remove everruns-runtime
    cargo add everruns everruns-host
    ```

    ```rust
    use everruns_host::{HostBackends, InProcessRuntimeBuilder};
    ```

  Every `everruns_runtime::` path maps to the identical `everruns_host::` path,
  with one rename: `RuntimeBackends` is `everruns_host::HostBackends`. The two
  deprecated legacy shims, `RuntimeMessageStore` and `EventBus`, have no
  replacement — canonical events are the only maintained write path, so use
  `everruns_host::EventLog` and `EventHistory` for history and
  `everruns_host::EventSink` and `EventReader` for observation and replay.

### Changed

- Chats is now core functionality for every organization: it is always present in navigation and search, requires no feature opt-in, and retains voice as a separately controlled capability.
- Knowledge-index sync embedding spend is now ledgered. `llm_generations.session_id`
  is nullable so org-attributed background inference has a home, and the reporting
  projection left-joins sessions so those rows reach `fact_llm_generation` with null
  session dimensions instead of being dropped. Sync spend is org-attributed but not
  debited against a session budget.
- `InMemoryAgenticLoop::seed_events` replays pre-recorded conversation
  envelopes into a fixture session's event log. It is the supported way to
  give an in-memory loop a prior conversation now that history projects from
  canonical events and `EventHistory` is read-only.

### What's Changed

- refactor(llmsim): extract publishable simulator crate by [@chaliy](https://github.com/chaliy)
- fix(framework): close provider adoption gaps ([#3189](https://github.com/everruns/everruns/pull/3189)) by [@chaliy](https://github.com/chaliy)
- docs(framework): publish execution architecture ([#3188](https://github.com/everruns/everruns/pull/3188)) by [@chaliy](https://github.com/chaliy)
- fix: resolve deep smoke test regressions ([#3187](https://github.com/everruns/everruns/pull/3187)) by [@chaliy](https://github.com/chaliy)
- refactor(framework): simplify coding agent setup ([#3186](https://github.com/everruns/everruns/pull/3186)) by [@chaliy](https://github.com/chaliy)
- ci(actions): move every action off the deprecated Node 20 runtime ([#3184](https://github.com/everruns/everruns/pull/3184)) by [@chaliy](https://github.com/chaliy)
- refactor(engine): split reason execution states ([#3185](https://github.com/everruns/everruns/pull/3185)) by [@chaliy](https://github.com/chaliy)
- refactor(framework): complete library boundary cleanup ([#3182](https://github.com/everruns/everruns/pull/3182)) by [@chaliy](https://github.com/chaliy)
- fix(docs): bump the nanoid override to the actual GHSA-2v37-7h3g-55p8 fix ([#3183](https://github.com/everruns/everruns/pull/3183)) by [@chaliy](https://github.com/chaliy)
- chore(knowledge): move active proposals into knowledge ([#3181](https://github.com/everruns/everruns/pull/3181)) by [@chaliy](https://github.com/chaliy)
- chore(agents): document PR evidence publishing ([#3180](https://github.com/everruns/everruns/pull/3180)) by [@chaliy](https://github.com/chaliy)
- fix(engine): retry stalls after reasoning output ([#3177](https://github.com/everruns/everruns/pull/3177)) by [@chaliy](https://github.com/chaliy)
- refactor(framework)!: make engines own session lifecycle ([#3179](https://github.com/everruns/everruns/pull/3179)) by [@chaliy](https://github.com/chaliy)
- refactor(engine): finish unified execution migration ([#3175](https://github.com/everruns/everruns/pull/3175)) by [@chaliy](https://github.com/chaliy)
- perf(build): speed up worktree validation ([#3176](https://github.com/everruns/everruns/pull/3176)) by [@chaliy](https://github.com/chaliy)
- fix(ci): prevent shell injection in release workflow ([#3174](https://github.com/everruns/everruns/pull/3174)) by [@chaliy](https://github.com/chaliy)
- refactor(engine): unify in-process and durable execution ([#3173](https://github.com/everruns/everruns/pull/3173)) by [@chaliy](https://github.com/chaliy)
- revert(scale): remove portable distributed engine ([#3172](https://github.com/everruns/everruns/pull/3172)) by [@chaliy](https://github.com/chaliy)
- feat(scale): add portable distributed engine by [@chaliy](https://github.com/chaliy)
- refactor(engine)!: own shared execution kernel ([#3170](https://github.com/everruns/everruns/pull/3170)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add engine-owned sessions ([#3167](https://github.com/everruns/everruns/pull/3167)) by [@chaliy](https://github.com/chaliy)
- refactor(example): use Framework workspace heads ([#3168](https://github.com/everruns/everruns/pull/3168)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add multi-head workspaces ([#3166](https://github.com/everruns/everruns/pull/3166)) by [@chaliy](https://github.com/chaliy)
- fix(ui): bump the nanoid override to the actual GHSA-2v37-7h3g-55p8 fix ([#3165](https://github.com/everruns/everruns/pull/3165)) by [@chaliy](https://github.com/chaliy)
- feat(embeddings): ledger index-sync embedding spend against the org ([#3163](https://github.com/everruns/everruns/pull/3163)) by [@chaliy](https://github.com/chaliy)
- fix(research): migrate lua eval from core exports ([#3164](https://github.com/everruns/everruns/pull/3164)) by [@chaliy](https://github.com/chaliy)
- fix(server): preserve credentialless gRPC provider type ([#3162](https://github.com/everruns/everruns/pull/3162)) by [@chaliy](https://github.com/chaliy)
- fix(research): replay seeded conversation events into the session log ([#3161](https://github.com/everruns/everruns/pull/3161)) by [@chaliy](https://github.com/chaliy)
- refactor(core)!: freeze the 0.18 neutral-kernel boundary ([#3160](https://github.com/everruns/everruns/pull/3160)) by [@chaliy](https://github.com/chaliy)
- refactor(host): move store-backed execution resolution ([#3159](https://github.com/everruns/everruns/pull/3159)) by [@chaliy](https://github.com/chaliy)
- refactor(core): split execution service contracts ([#3158](https://github.com/everruns/everruns/pull/3158)) by [@chaliy](https://github.com/chaliy)
- refactor(core): isolate TLS and transport dependencies ([#3157](https://github.com/everruns/everruns/pull/3157)) by [@chaliy](https://github.com/chaliy)
- refactor(storage): move in-memory backends to owners by [@chaliy](https://github.com/chaliy)
- refactor(core): move concrete capabilities out of kernel by [@chaliy](https://github.com/chaliy)
- chore(knowledge): close EVE-897 and record why the context bag stays ([#3154](https://github.com/everruns/everruns/pull/3154)) by [@chaliy](https://github.com/chaliy)
- chore(ui): retire the Dashboard as a top-level page ([#3153](https://github.com/everruns/everruns/pull/3153)) by [@chaliy](https://github.com/chaliy)
- fix(research): finish the HostComposition rename in lua-vs-bash ([#3152](https://github.com/everruns/everruns/pull/3152)) by [@chaliy](https://github.com/chaliy)
- feat(openresponses): report per-call cost for compaction requests ([#3150](https://github.com/everruns/everruns/pull/3150)) by [@chaliy](https://github.com/chaliy)
- feat(ui): breadcrumbs name their navigation group ([#3151](https://github.com/everruns/everruns/pull/3151)) by [@chaliy](https://github.com/chaliy)
- feat(embeddings): meter retrieval embedding spend ([#3149](https://github.com/everruns/everruns/pull/3149)) by [@chaliy](https://github.com/chaliy)
- docs(how-to): add the 0.18 migration guide for direct core consumers ([#3148](https://github.com/everruns/everruns/pull/3148)) by [@chaliy](https://github.com/chaliy)
- refactor(core): drop the dead embedded-platform-docs feature from the kernel ([#3146](https://github.com/everruns/everruns/pull/3146)) by [@chaliy](https://github.com/chaliy)
- refactor(context): resolve the session mutator as a typed extension ([#3147](https://github.com/everruns/everruns/pull/3147)) by [@chaliy](https://github.com/chaliy)
- refactor(core): drop the vestigial sqlx feature from the kernel ([#3145](https://github.com/everruns/everruns/pull/3145)) by [@chaliy](https://github.com/chaliy)
- chore(knowledge): close EVE-880 — three session families stay in the kernel ([#3143](https://github.com/everruns/everruns/pull/3143)) by [@chaliy](https://github.com/chaliy)
- test(llm): skip explicit OpenRouter credit ceilings ([#3144](https://github.com/everruns/everruns/pull/3144)) by [@chaliy](https://github.com/chaliy)
- test(llm): separate sampling misses from contract failures ([#3142](https://github.com/everruns/everruns/pull/3142)) by [@chaliy](https://github.com/chaliy)
- refactor(core): move background tool runs out of the kernel ([#3140](https://github.com/everruns/everruns/pull/3140)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): stop export-openapi truncating the committed spec ([#3139](https://github.com/everruns/everruns/pull/3139)) by [@chaliy](https://github.com/chaliy)
- refactor(context): resolve the session SQL store as a typed extension ([#3138](https://github.com/everruns/everruns/pull/3138)) by [@chaliy](https://github.com/chaliy)
- refactor(composition): move the composition root out of the kernel ([#3137](https://github.com/everruns/everruns/pull/3137)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move the Workspace and managed session sandbox records out of the kernel ([#3133](https://github.com/everruns/everruns/pull/3133)) by [@chaliy](https://github.com/chaliy)
- feat(agents): protect built-in agents at the command layer ([#3136](https://github.com/everruns/everruns/pull/3136)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump @astrojs/markdown-satteri from 0.3.4 to 0.3.5 in /apps/docs ([#3125](https://github.com/everruns/everruns/pull/3125)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump distroless/cc-debian12 from `fccdbb0` to `adcd20c` in /docker ([#3124](https://github.com/everruns/everruns/pull/3124)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump distroless/cc-debian12 from `fccdbb0` to `adcd20c` in /crates/server ([#3122](https://github.com/everruns/everruns/pull/3122)) by [@dependabot](https://github.com/dependabot)
- chore(deps): take syn 3, hold buffa and js-yaml majors ([#3135](https://github.com/everruns/everruns/pull/3135)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump distroless/cc-debian12 from `fccdbb0` to `adcd20c` in /crates/worker ([#3123](https://github.com/everruns/everruns/pull/3123)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump marked from 18.0.6 to 18.0.9 in /apps/docs ([#3127](https://github.com/everruns/everruns/pull/3127)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump @astrojs/check from 0.9.9 to 0.9.10 in /apps/docs ([#3128](https://github.com/everruns/everruns/pull/3128)) by [@dependabot](https://github.com/dependabot)
- ci: stop third-party apt repo failures from reddening main ([#3134](https://github.com/everruns/everruns/pull/3134)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): move session-service capabilities out of the kernel ([#3132](https://github.com/everruns/everruns/pull/3132)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): move portable policy builtins out of core ([#3131](https://github.com/everruns/everruns/pull/3131)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): move hosted knowledge, delegation and platform capabilities out of core ([#3111](https://github.com/everruns/everruns/pull/3111)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): move environment implementations out of core ([#3119](https://github.com/everruns/everruns/pull/3119)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move connector, OAuth, credential and system-email infrastructure out of core ([#3120](https://github.com/everruns/everruns/pull/3120)) by [@chaliy](https://github.com/chaliy)
- fix(core): fail closed on unknown capability registration gates ([#3121](https://github.com/everruns/everruns/pull/3121)) by [@chaliy](https://github.com/chaliy)
- feat(models): use platform default fallback ([#3114](https://github.com/everruns/everruns/pull/3114)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move eval, observer and feature-management records out of core ([#3118](https://github.com/everruns/everruns/pull/3118)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move the persisted Session aggregate out of core ([#3117](https://github.com/everruns/everruns/pull/3117)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move Harness records and built-in provisioning out of core ([#3116](https://github.com/everruns/everruns/pull/3116)) by [@chaliy](https://github.com/chaliy)
- fix(llm-tests): skip subagent live test on provider quota exhaustion ([#3115](https://github.com/everruns/everruns/pull/3115)) by [@chaliy](https://github.com/chaliy)
- feat(ui): separate session recording projections ([#3113](https://github.com/everruns/everruns/pull/3113)) by [@chaliy](https://github.com/chaliy)
- feat(chats): add chat pinning by [@chaliy](https://github.com/chaliy)
- refactor(providers): remove provider crates' direct dependency on core ([#3110](https://github.com/everruns/everruns/pull/3110)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move Agent and AgentVersion persistence records out of core ([#3109](https://github.com/everruns/everruns/pull/3109)) by [@chaliy](https://github.com/chaliy)
- refactor(observability): move telemetry and event-listener implementations out of core ([#3108](https://github.com/everruns/everruns/pull/3108)) by [@chaliy](https://github.com/chaliy)
- refactor(test-support): move llmsim, in-memory loop and fake capabilities out of core ([#3107](https://github.com/everruns/everruns/pull/3107)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): establish one neutral capability contract for Framework and product ([#3106](https://github.com/everruns/everruns/pull/3106)) by [@chaliy](https://github.com/chaliy)
- fix(examples): repair standalone crates after session-events and DriverId API drift ([#3105](https://github.com/everruns/everruns/pull/3105)) by [@chaliy](https://github.com/chaliy)
- refactor(host): replace stored record inputs with resolved execution snapshot ([#3103](https://github.com/everruns/everruns/pull/3103)) by [@chaliy](https://github.com/chaliy)
- fix(host): make the canonical EventLog SPI externally implementable ([#3102](https://github.com/everruns/everruns/pull/3102)) by [@chaliy](https://github.com/chaliy)
- refactor(0.18)!: delete the everruns-runtime compatibility crate ([#3101](https://github.com/everruns/everruns/pull/3101)) by [@chaliy](https://github.com/chaliy)
- fix(chat): make Chats unconditional ([#3100](https://github.com/everruns/everruns/pull/3100)) by [@chaliy](https://github.com/chaliy)
- test: use obvious credential fixtures ([#3099](https://github.com/everruns/everruns/pull/3099)) by [@chaliy](https://github.com/chaliy)
- fix(release): correct everruns-local publish dependency map ([#3098](https://github.com/everruns/everruns/pull/3098)) by [@chaliy](https://github.com/chaliy)

## [0.17.26] - 2026-08-10

### Highlights

- **Live, steerable sessions** - Sessions are now live and steerable, with canonical session events, session history and resume, and a session work and wake API ([#3095](https://github.com/everruns/everruns/pull/3095), [#3090](https://github.com/everruns/everruns/pull/3090), [#3089](https://github.com/everruns/everruns/pull/3089), [#3083](https://github.com/everruns/everruns/pull/3083)).
- **Sessions as the operational list** - The Sessions surface was rebuilt as the operational list, recording session source and offering a filterable list with facet counts ([#3078](https://github.com/everruns/everruns/pull/3078), [#3073](https://github.com/everruns/everruns/pull/3073)).
- **Advanced capability authoring** - New advanced capability authoring API with typed lifecycle hooks, a workspace access policy, unified capability configuration, and separated provider and model configuration ([#3084](https://github.com/everruns/everruns/pull/3084), [#3086](https://github.com/everruns/everruns/pull/3086), [#3085](https://github.com/everruns/everruns/pull/3085), [#3096](https://github.com/everruns/everruns/pull/3096), [#3093](https://github.com/everruns/everruns/pull/3093)).
- **Daytona workspace recovery** - Sandboxes recover Daytona workspaces after instance loss ([#3092](https://github.com/everruns/everruns/pull/3092)).

### What's Changed

- feat(framework)!: unify capability configuration ([#3096](https://github.com/everruns/everruns/pull/3096)) by [@chaliy](https://github.com/chaliy)
- docs(runtime): add 0.17 migration path ([#3091](https://github.com/everruns/everruns/pull/3091)) by [@chaliy](https://github.com/chaliy)
- feat(framework): make sessions live and steerable ([#3095](https://github.com/everruns/everruns/pull/3095)) by [@chaliy](https://github.com/chaliy)
- feat(sandbox): recover Daytona workspaces after instance loss ([#3092](https://github.com/everruns/everruns/pull/3092)) by [@chaliy](https://github.com/chaliy)
- fix(framework): bundle simulated error provider ([#3094](https://github.com/everruns/everruns/pull/3094)) by [@chaliy](https://github.com/chaliy)
- feat(framework)!: separate provider and model configuration ([#3093](https://github.com/everruns/everruns/pull/3093)) by [@chaliy](https://github.com/chaliy)
- feat(framework): expose canonical session events ([#3090](https://github.com/everruns/everruns/pull/3090)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add session history and resume ([#3089](https://github.com/everruns/everruns/pull/3089)) by [@chaliy](https://github.com/chaliy)
- refactor(runtime): migrate first-party consumers off compatibility crate ([#3088](https://github.com/everruns/everruns/pull/3088)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add workspace access policy ([#3085](https://github.com/everruns/everruns/pull/3085)) by [@chaliy](https://github.com/chaliy)
- refactor(runtime): make runtime a host compatibility adapter ([#3087](https://github.com/everruns/everruns/pull/3087)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add typed lifecycle hooks ([#3086](https://github.com/everruns/everruns/pull/3086)) by [@chaliy](https://github.com/chaliy)
- feat(framework): add advanced capability authoring API ([#3084](https://github.com/everruns/everruns/pull/3084)) by [@chaliy](https://github.com/chaliy)
- feat(everruns): add session work and wake API ([#3083](https://github.com/everruns/everruns/pull/3083)) by [@chaliy](https://github.com/chaliy)
- refactor(host): extract shared orchestration ([#3082](https://github.com/everruns/everruns/pull/3082)) by [@chaliy](https://github.com/chaliy)
- feat(framework): expose typed session identity by [@chaliy](https://github.com/chaliy)
- docs(framework): establish primary library documentation by [@chaliy](https://github.com/chaliy)
- feat(everruns): close application runtime API gaps ([#3079](https://github.com/everruns/everruns/pull/3079)) by [@chaliy](https://github.com/chaliy)
- fix(features): hide disabled surfaces ([#3075](https://github.com/everruns/everruns/pull/3075)) by [@chaliy](https://github.com/chaliy)
- feat(ui): rebuild Sessions as the operational list ([#3078](https://github.com/everruns/everruns/pull/3078)) by [@chaliy](https://github.com/chaliy)
- fix(security): resolve open Dependabot advisories in ui and docs ([#3076](https://github.com/everruns/everruns/pull/3076)) by [@chaliy](https://github.com/chaliy)
- feat(examples): port public everruns examples ([#3074](https://github.com/everruns/everruns/pull/3074)) by [@chaliy](https://github.com/chaliy)
- fix(ui): regenerate stale API types and gate the UI job on the spec ([#3077](https://github.com/everruns/everruns/pull/3077)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): redact reflected bound credentials ([#3062](https://github.com/everruns/everruns/pull/3062)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): record session source, filterable list with facet counts ([#3073](https://github.com/everruns/everruns/pull/3073)) by [@chaliy](https://github.com/chaliy)
- chore(macros): move crate to concise path ([#3070](https://github.com/everruns/everruns/pull/3070)) by [@chaliy](https://github.com/chaliy)
- fix(security): gate machine-payment custody surfaces ([#3069](https://github.com/everruns/everruns/pull/3069)) by [@chaliy](https://github.com/chaliy)
- refactor(provider): separate providers from drivers ([#3072](https://github.com/everruns/everruns/pull/3072)) by [@chaliy](https://github.com/chaliy)

## [0.17.25] - 2026-08-09

### What's Changed

- fix(docker): install perl and make in the unified builder so git2's `vendored-openssl` build succeeds ([#3071](https://github.com/everruns/everruns/pull/3071)) by [@chaliy](https://github.com/chaliy)
- fix(docker): copy `pnpm-workspace.yaml` into the UI image build so the frozen install finds the `overrides` config ([#3071](https://github.com/everruns/everruns/pull/3071)) by [@chaliy](https://github.com/chaliy)

## [0.17.24] - 2026-08-09

### Highlights

- **Publishable `everruns` library** - A new value-first `everruns` facade crate lets you build agents in Rust with `AgentBuilder`, async functions and closures as tools (including the `#[everruns::tool]` macro), multi-turn `Session::run()`, session events and cancellation, an `openai` feature with `OpenAI::from_env()`, and JSONL session save/resume ([#3032](https://github.com/everruns/everruns/pull/3032), [#3033](https://github.com/everruns/everruns/pull/3033), [#3034](https://github.com/everruns/everruns/pull/3034), [#3038](https://github.com/everruns/everruns/pull/3038), [#3046](https://github.com/everruns/everruns/pull/3046), [#3045](https://github.com/everruns/everruns/pull/3045), [#3048](https://github.com/everruns/everruns/pull/3048)).
- **Chat-centric UI** - Chats are now the landing surface with a thread list in the sidebar and a regrouped navigation (Chats · Operational · Building · Registries · Quality); you can address participants with @mentions, session detail becomes a read-only recording you can Fork into a chat, and the agent and harness editors were restructured ([#3061](https://github.com/everruns/everruns/pull/3061), [#3057](https://github.com/everruns/everruns/pull/3057), [#3030](https://github.com/everruns/everruns/pull/3030), [#3059](https://github.com/everruns/everruns/pull/3059), [#3029](https://github.com/everruns/everruns/pull/3029)).
- **Catalog-backed Platform capability** - Platform Chat now drives Everruns through a built-in `platform` capability with the same catalog-backed `discover`, `query`, and `execute` contracts as the MCP endpoint, so it can discover models, set an agent's default model, manage MCP resources, and create Agent Triggers directly ([#2986](https://github.com/everruns/everruns/pull/2986)).
- **Secure MCP credential bindings** - Agents can now bind MCP server credentials through a secure store instead of inline configuration ([#3051](https://github.com/everruns/everruns/pull/3051)).
- **Agent Plugins v1** - Everruns installs portable canonical Agent Plugins v1 packages (`plugin.json`/`mcp.json`, validated against the v1 schema) alongside the existing Claude, Codex, and Cursor plugin layouts ([#2953](https://github.com/everruns/everruns/pull/2953)).

### What's Changed

- refactor(platform): move identity family out of core ([#3060](https://github.com/everruns/everruns/pull/3060)) by [@chaliy](https://github.com/chaliy)
- feat(ui): Chats as the landing surface — thread list in the sidebar, thread detail ([#3061](https://github.com/everruns/everruns/pull/3061)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): session detail becomes a read-only recording with Fork into chat ([#3059](https://github.com/everruns/everruns/pull/3059)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move app and agent-trigger records out of core ([#3058](https://github.com/everruns/everruns/pull/3058)) by [@chaliy](https://github.com/chaliy)
- feat(ui): regroup the sidebar into Chats · Operational · Building · Registries · Quality ([#3057](https://github.com/everruns/everruns/pull/3057)) by [@chaliy](https://github.com/chaliy)
- fix(release): publish everruns-engine and everruns-macros ([#3056](https://github.com/everruns/everruns/pull/3056)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move PlatformStore and hosted management capabilities out of core ([#3053](https://github.com/everruns/everruns/pull/3053)) by [@chaliy](https://github.com/chaliy)
- fix(ui): reorder agent detail tabs ([#3055](https://github.com/everruns/everruns/pull/3055)) by [@chaliy](https://github.com/chaliy)
- refactor(runtime): drive InProcessRuntime turns through everruns-engine ([#3052](https://github.com/everruns/everruns/pull/3052)) by [@chaliy](https://github.com/chaliy)
- feat(security): add secure MCP credential bindings ([#3051](https://github.com/everruns/everruns/pull/3051)) by [@chaliy](https://github.com/chaliy)
- fix(deps): update quinn-proto security patch ([#3050](https://github.com/everruns/everruns/pull/3050)) by [@chaliy](https://github.com/chaliy)
- refactor(engine): extract the turn planner into everruns-engine ([#3049](https://github.com/everruns/everruns/pull/3049)) by [@chaliy](https://github.com/chaliy)
- feat(library): add JSONL session save and resume ([#3048](https://github.com/everruns/everruns/pull/3048)) by [@chaliy](https://github.com/chaliy)
- refactor(example): migrate coding-cli to depend only on everruns ([#3047](https://github.com/everruns/everruns/pull/3047)) by [@chaliy](https://github.com/chaliy)
- feat(macros): implement #[everruns::tool] for typed async functions ([#3046](https://github.com/everruns/everruns/pull/3046)) by [@chaliy](https://github.com/chaliy)
- feat(library): expose session events and cancellation ([#3045](https://github.com/everruns/everruns/pull/3045)) by [@chaliy](https://github.com/chaliy)
- fix(platform): ground chat resource discovery ([#3040](https://github.com/everruns/everruns/pull/3040)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): move payment, reporting, and audit records out of everruns-core ([#3044](https://github.com/everruns/everruns/pull/3044)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): restore connected remote MCP tools ([#3043](https://github.com/everruns/everruns/pull/3043)) by [@chaliy](https://github.com/chaliy)
- feat(library): add `openai` feature and `OpenAI::from_env()` model configuration ([#3042](https://github.com/everruns/everruns/pull/3042)) by [@chaliy](https://github.com/chaliy)
- fix(chat): stabilize tool activity narration ([#3037](https://github.com/everruns/everruns/pull/3037)) by [@chaliy](https://github.com/chaliy)
- fix(reporting): wire MCP report queries ([#3039](https://github.com/everruns/everruns/pull/3039)) by [@chaliy](https://github.com/chaliy)
- feat(library): accept async Rust functions and closures as agent tools ([#3038](https://github.com/everruns/everruns/pull/3038)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify agent version history by [@chaliy](https://github.com/chaliy)
- refactor(platform): create everruns-platform crate and move organization/principal models out of core ([#3036](https://github.com/everruns/everruns/pull/3036)) by [@chaliy](https://github.com/chaliy)
- feat(library): add `Agent::session()` and multi-turn `Session::run()` ([#3034](https://github.com/everruns/everruns/pull/3034)) by [@chaliy](https://github.com/chaliy)
- fix(chat): render markdown in work logs ([#3035](https://github.com/everruns/everruns/pull/3035)) by [@chaliy](https://github.com/chaliy)
- feat(library): add value-first `everruns::AgentBuilder` ([#3033](https://github.com/everruns/everruns/pull/3033)) by [@chaliy](https://github.com/chaliy)
- feat(cli): manage composition resources ([#3031](https://github.com/everruns/everruns/pull/3031)) by [@chaliy](https://github.com/chaliy)
- feat(library): add the publishable `everruns` facade crate ([#3032](https://github.com/everruns/everruns/pull/3032)) by [@chaliy](https://github.com/chaliy)
- fix(chat): restore slash commands and global ownership ([#3027](https://github.com/everruns/everruns/pull/3027)) by [@chaliy](https://github.com/chaliy)
- feat(chat): address participants with mentions by [@chaliy](https://github.com/chaliy)
- feat(ui): restructure agent and harness editors ([#3029](https://github.com/everruns/everruns/pull/3029)) by [@chaliy](https://github.com/chaliy)
- fix(dev): preserve local encryption key across startups ([#3028](https://github.com/everruns/everruns/pull/3028)) by [@chaliy](https://github.com/chaliy)
- chore(dev): canonicalize agent startup ([#3026](https://github.com/everruns/everruns/pull/3026)) by [@chaliy](https://github.com/chaliy)
- fix(platform): make agent provisioning reliable ([#3022](https://github.com/everruns/everruns/pull/3022)) by [@chaliy](https://github.com/chaliy)
- fix(evals): bound async dataset exports ([#3004](https://github.com/everruns/everruns/pull/3004)) by [@chaliy](https://github.com/chaliy)
- fix(api): hide private memory from workspace grep ([#3025](https://github.com/everruns/everruns/pull/3025)) by [@chaliy](https://github.com/chaliy)
- fix(server): cascade org seed resources on rollback ([#3002](https://github.com/everruns/everruns/pull/3002)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): exclude private memory before grep ([#2991](https://github.com/everruns/everruns/pull/2991)) by [@chaliy](https://github.com/chaliy)
- fix(providers): ignore disabled provider credentials ([#2997](https://github.com/everruns/everruns/pull/2997)) by [@chaliy](https://github.com/chaliy)
- fix(settings): persist organization default model ([#3020](https://github.com/everruns/everruns/pull/3020)) by [@chaliy](https://github.com/chaliy)
- fix(agents): surface terminal diagnostic failures ([#3023](https://github.com/everruns/everruns/pull/3023)) by [@chaliy](https://github.com/chaliy)
- fix(harnesses): show inheritance on summary cards ([#3021](https://github.com/everruns/everruns/pull/3021)) by [@chaliy](https://github.com/chaliy)
- fix(local): preserve wakes after host disconnect ([#3003](https://github.com/everruns/everruns/pull/3003)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): read bash commands in progress guard ([#2998](https://github.com/everruns/everruns/pull/2998)) by [@chaliy](https://github.com/chaliy)
- fix(provider): harden model discovery fallback ([#3001](https://github.com/everruns/everruns/pull/3001)) by [@chaliy](https://github.com/chaliy)
- fix(server): reject unsafe OKF export paths ([#3000](https://github.com/everruns/everruns/pull/3000)) by [@chaliy](https://github.com/chaliy)
- fix(authz): restrict Platform Chat turns to owner ([#3008](https://github.com/everruns/everruns/pull/3008)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add Slate component showcase ([#3024](https://github.com/everruns/everruns/pull/3024)) by [@chaliy](https://github.com/chaliy)
- chore(deps): refresh frontend security overrides ([#3017](https://github.com/everruns/everruns/pull/3017)) by [@chaliy](https://github.com/chaliy)
- fix(ui): make page layouts responsive ([#3018](https://github.com/everruns/everruns/pull/3018)) by [@chaliy](https://github.com/chaliy)
- fix(ui): keep local user actions in sidebar ([#3019](https://github.com/everruns/everruns/pull/3019)) by [@chaliy](https://github.com/chaliy)
- feat(ui): standardize entity ID affordances ([#3016](https://github.com/everruns/everruns/pull/3016)) by [@chaliy](https://github.com/chaliy)
- fix(dev): propagate auth mode across start-all ([#3015](https://github.com/everruns/everruns/pull/3015)) by [@chaliy](https://github.com/chaliy)
- fix(ui): refine responsive mobile chrome ([#3014](https://github.com/everruns/everruns/pull/3014)) by [@chaliy](https://github.com/chaliy)
- fix(knowledge): normalize GitHub repository sources ([#3013](https://github.com/everruns/everruns/pull/3013)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove duplicate compact masthead actions by [@chaliy](https://github.com/chaliy)
- test(evals): prove platform capability behavior ([#3011](https://github.com/everruns/everruns/pull/3011)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): use Opus 5 for extended-thinking reasoning assertion ([#3009](https://github.com/everruns/everruns/pull/3009)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): widen extended-thinking retry budget to 6 ([#3007](https://github.com/everruns/everruns/pull/3007)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): require captured reasoning before accepting extended-thinking attempt ([#3006](https://github.com/everruns/everruns/pull/3006)) by [@chaliy](https://github.com/chaliy)
- fix(web-fetch): require egress for policy-scoped crawl ([#2988](https://github.com/everruns/everruns/pull/2988)) by [@chaliy](https://github.com/chaliy)
- fix(evals): bound external import fan-out ([#2995](https://github.com/everruns/everruns/pull/2995)) by [@chaliy](https://github.com/chaliy)
- fix(auth): reject cross-provider OAuth auto-linking ([#2985](https://github.com/everruns/everruns/pull/2985)) by [@chaliy](https://github.com/chaliy)
- fix(filesystem): bound serialized grep context ([#2989](https://github.com/everruns/everruns/pull/2989)) by [@chaliy](https://github.com/chaliy)
- fix(storage): reclaim superseded file blob revisions ([#2990](https://github.com/everruns/everruns/pull/2990)) by [@chaliy](https://github.com/chaliy)
- fix(security): omit sensitive tool narration input ([#2987](https://github.com/everruns/everruns/pull/2987)) by [@chaliy](https://github.com/chaliy)
- fix(provider): keep SSRF IP blocking global ([#2984](https://github.com/everruns/everruns/pull/2984)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): enforce prompt hooks on wake messages ([#2993](https://github.com/everruns/everruns/pull/2993)) by [@chaliy](https://github.com/chaliy)
- fix(core): fail closed when detached cancel cannot signal peer ([#2992](https://github.com/everruns/everruns/pull/2992)) by [@chaliy](https://github.com/chaliy)
- fix(core): resolve input before displaying paths ([#2996](https://github.com/everruns/everruns/pull/2996)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): bind OAuth resource to server origin ([#2983](https://github.com/everruns/everruns/pull/2983)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redact secret_store values in trajectory details ([#2982](https://github.com/everruns/everruns/pull/2982)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump js-yaml from 4.3.0 to 4.3.1 in /apps/docs ([#2956](https://github.com/everruns/everruns/pull/2956)) by [@app/dependabot](https://github.com/apps/dependabot)
- feat(plugins): support Agent Plugins v1 ([#2953](https://github.com/everruns/everruns/pull/2953)) by [@chaliy](https://github.com/chaliy)
- fix(ui): align compact actions with masthead icon ([#2999](https://github.com/everruns/everruns/pull/2999)) by [@chaliy](https://github.com/chaliy)
- fix(knowledge): ingest new indexes automatically ([#2974](https://github.com/everruns/everruns/pull/2974)) by [@chaliy](https://github.com/chaliy)
- feat(platform): add catalog-backed platform capability ([#2986](https://github.com/everruns/everruns/pull/2986)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to 0.16.0 ([#2980](https://github.com/everruns/everruns/pull/2980)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prioritize detail actions responsively ([#2994](https://github.com/everruns/everruns/pull/2994)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): stabilize capability assignment ([#2981](https://github.com/everruns/everruns/pull/2981)) by [@chaliy](https://github.com/chaliy)
- docs(guardrails): add capability page and refresh gallery blurb ([#2979](https://github.com/everruns/everruns/pull/2979)) by [@chaliy](https://github.com/chaliy)
- fix(ui): make page mastheads responsive ([#2978](https://github.com/everruns/everruns/pull/2978)) by [@chaliy](https://github.com/chaliy)
- fix(models): configure embedding models separately ([#2975](https://github.com/everruns/everruns/pull/2975)) by [@chaliy](https://github.com/chaliy)
- fix(ui): reconcile marketplace plugin installs ([#2977](https://github.com/everruns/everruns/pull/2977)) by [@chaliy](https://github.com/chaliy)
- fix(ui): move agent diagnostics into edit rail ([#2976](https://github.com/everruns/everruns/pull/2976)) by [@chaliy](https://github.com/chaliy)
- fix(release): publish everruns-meta crate ([#2972](https://github.com/everruns/everruns/pull/2972)) by [@chaliy](https://github.com/chaliy)
- fix(knowledge-indexes): surface actionable issue states ([#2973](https://github.com/everruns/everruns/pull/2973)) by [@chaliy](https://github.com/chaliy)
- chore(knowledge): migrate specs to OKF v0.2 ([#2950](https://github.com/everruns/everruns/pull/2950)) by [@chaliy](https://github.com/chaliy)
- fix(ui): contain agent identity card content ([#2971](https://github.com/everruns/everruns/pull/2971)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add chip-based tag editor ([#2970](https://github.com/everruns/everruns/pull/2970)) by [@chaliy](https://github.com/chaliy)
- fix(ui): correct knowledge index form validation ([#2969](https://github.com/everruns/everruns/pull/2969)) by [@chaliy](https://github.com/chaliy)
- fix(ui): make model rows responsive ([#2968](https://github.com/everruns/everruns/pull/2968)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): show user names for participants ([#2966](https://github.com/everruns/everruns/pull/2966)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): install plugins from larger repositories ([#2964](https://github.com/everruns/everruns/pull/2964)) by [@chaliy](https://github.com/chaliy)
- fix(chat): summarize repeated tool activity ([#2960](https://github.com/everruns/everruns/pull/2960)) by [@chaliy](https://github.com/chaliy)
- fix(tools): keep bash schema loaded ([#2959](https://github.com/everruns/everruns/pull/2959)) by [@chaliy](https://github.com/chaliy)
- feat(provider): search model catalogs across providers ([#2961](https://github.com/everruns/everruns/pull/2961)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show resolved default chat model ([#2958](https://github.com/everruns/everruns/pull/2958)) by [@chaliy](https://github.com/chaliy)
- feat(core): serializable TurnState with pure transitions ([#2952](https://github.com/everruns/everruns/pull/2952)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show tool execution in session trajectory ([#2963](https://github.com/everruns/everruns/pull/2963)) by [@chaliy](https://github.com/chaliy)
- fix(ui): move agent checks into edit tab ([#2962](https://github.com/everruns/everruns/pull/2962)) by [@chaliy](https://github.com/chaliy)

## [0.17.23] - 2026-08-06

### Highlights

- **New model providers** - Meta Model API and Muse Spark are now available as first-class providers ([#2941](https://github.com/everruns/everruns/pull/2941)).
- **Host-managed providers** - Org admins now see host-provisioned provider rows as read-only, making platform-managed providers visible without being editable ([#2944](https://github.com/everruns/everruns/pull/2944)).

### What's Changed

- feat(provider): add Meta Model API and Muse Spark ([#2941](https://github.com/everruns/everruns/pull/2941)) by [@chaliy](https://github.com/chaliy)
- feat(providers): host-managed provider rows (read-only to org admins) ([#2944](https://github.com/everruns/everruns/pull/2944)) by [@chaliy](https://github.com/chaliy)
- feat(embedding): org-initialization hook for embedder-provisioned org resources ([#2943](https://github.com/everruns/everruns/pull/2943)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump brace-expansion and undici to patched versions (security) ([#2945](https://github.com/everruns/everruns/pull/2945)) by [@chaliy](https://github.com/chaliy)
- chore(ship): prompt for PR evidence in the ship skill ([#2942](https://github.com/everruns/everruns/pull/2942)) by [@chaliy](https://github.com/chaliy)

## [0.17.22] - 2026-08-06

### Highlights

- **Automatic interrupted-turn recovery** - Agent turns now classify provider failures, recover safely from transient transport, overload, 5xx, and stream-stall failures within bounded retry budgets, preserve completed tool effects, and surface permanent failures as precise resumable errors ([#2937](https://github.com/everruns/everruns/pull/2937)).
- **Safer interactive tools** - Tool calls can now request host approval and receive cooperative cancellation through `ToolContext`, giving hosts explicit control without abandoning in-flight work ([#2931](https://github.com/everruns/everruns/pull/2931), [#2938](https://github.com/everruns/everruns/pull/2938)).
- **Model-backed secret screening** - Gallery presets can now add a judge guardrail that catches likely secret leakage beyond deterministic pattern matching ([#2932](https://github.com/everruns/everruns/pull/2932)).

### What's Changed

- fix(runtime): recover interrupted provider turns ([#2937](https://github.com/everruns/everruns/pull/2937)) by [@chaliy](https://github.com/chaliy)
- feat(tools): cooperative cancellation token on ToolContext ([#2938](https://github.com/everruns/everruns/pull/2938)) by [@chaliy](https://github.com/chaliy)
- perf(anthropic): make prompt caching incremental ([#2934](https://github.com/everruns/everruns/pull/2934)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): interactive tool-approval gate ([#2931](https://github.com/everruns/everruns/pull/2931)) by [@chaliy](https://github.com/chaliy)
- feat(tools): host-owned metadata hatch on tool hints and capabilities ([#2936](https://github.com/everruns/everruns/pull/2936)) by [@chaliy](https://github.com/chaliy)
- feat(guardrails): add model-backed secret-leak judge gallery preset ([#2932](https://github.com/everruns/everruns/pull/2932)) by [@chaliy](https://github.com/chaliy)

## [0.17.21] - 2026-08-06

### Highlights

- **Cumulative-cost context checkpoints** - Long tool trajectories now compact before context-window pressure when cumulative uncached input or raw tool results become costly, while preserving lossless history and query access ([#2933](https://github.com/everruns/everruns/pull/2933)).
- **Lower tool context overhead** - Deferred tools now keep compact permissive schemas until revealed, reducing the production tool-list payload by 65% while preserving provider-agnostic structured tool calling ([#2935](https://github.com/everruns/everruns/pull/2935)).

### What's Changed

- feat(compaction): trigger checkpoints from cumulative cost ([#2933](https://github.com/everruns/everruns/pull/2933)) by [@chaliy](https://github.com/chaliy)
- perf(tool-search): compact deferred schemas ([#2935](https://github.com/everruns/everruns/pull/2935)) by [@chaliy](https://github.com/chaliy)

## [0.17.20] - 2026-08-06

### Highlights

- **Linked OAuth accounts** - Verified OAuth sign-ins now link to an existing account with a matching email instead of creating a duplicate ([#2914](https://github.com/everruns/everruns/pull/2914)).
- **Operator SSRF allowlist** - Self-hosted deployments can exempt specific CIDR ranges from SSRF blocking (`EVERRUNS_SSRF_ALLOW_CIDRS`), making in-cluster MCP servers reachable without exposing them publicly ([#2923](https://github.com/everruns/everruns/pull/2923)).
- **Auth reliability** - Sessions and org selection now persist across token refresh, and MCP OAuth consent redirects pass the CSP form-action policy ([#2919](https://github.com/everruns/everruns/pull/2919), [#2918](https://github.com/everruns/everruns/pull/2918)).

### What's Changed

- fix(sessions): mount registry skills referenced as skill capabilities ([#2927](https://github.com/everruns/everruns/pull/2927)) by [@chaliy](https://github.com/chaliy)
- feat(security): operator CIDR allowlist for SSRF validation ([#2923](https://github.com/everruns/everruns/pull/2923)) by [@shbodya](https://github.com/shbodya)
- chore(deps): bump the npm_and_yarn group across 1 directory with 2 updates ([#2925](https://github.com/everruns/everruns/pull/2925)) by [@app/dependabot](https://github.com/apps/dependabot)
- chore(deps): bump the cargo group with 3 updates ([#2926](https://github.com/everruns/everruns/pull/2926)) by [@app/dependabot](https://github.com/apps/dependabot)
- fix(deps): bump fast-uri and postcss to clear security advisories ([#2924](https://github.com/everruns/everruns/pull/2924)) by [@chaliy](https://github.com/chaliy)
- fix(integrations): fall back to Azure OpenAI credentials for image generation ([#2922](https://github.com/everruns/everruns/pull/2922)) by [@shbodya](https://github.com/shbodya)
- fix(skills): allow restoring archived skills via status-only update ([#2921](https://github.com/everruns/everruns/pull/2921)) by [@shbodya](https://github.com/shbodya)
- fix(auth): keep session and org selection across token refresh ([#2919](https://github.com/everruns/everruns/pull/2919)) by [@shbodya](https://github.com/shbodya)
- fix(server): allow MCP OAuth consent redirect through CSP form-action ([#2918](https://github.com/everruns/everruns/pull/2918)) by [@shbodya](https://github.com/shbodya)
- feat(docker): add authentication environment variables to docker-compose ([#2917](https://github.com/everruns/everruns/pull/2917)) by [@shbodya](https://github.com/shbodya)
- chore: green main CI (OpenAI credit-exhaustion skip) + patch brace-expansion DoS (GHSA-mh99-v99m-4gvg) ([#2916](https://github.com/everruns/everruns/pull/2916)) by [@chaliy](https://github.com/chaliy)
- fix(provider): retry provider stream stalls via shared transient path ([#2915](https://github.com/everruns/everruns/pull/2915)) by [@chaliy](https://github.com/chaliy)
- feat(auth): link verified OAuth accounts ([#2914](https://github.com/everruns/everruns/pull/2914)) by [@chaliy](https://github.com/chaliy)

## [0.17.19] - 2026-07-31

### Highlights

- **Safer coding-agent file edits** - Exact multi-hunk edits can now rebase over unrelated concurrent changes while conflicts remain atomic and leave files untouched ([#2912](https://github.com/everruns/everruns/pull/2912)).

### What's Changed

- fix(fs): safely rebase exact stale edits ([#2912](https://github.com/everruns/everruns/pull/2912)) by [@chaliy](https://github.com/chaliy)
- fix(core): detect read_file images by content ([#2911](https://github.com/everruns/everruns/pull/2911)) by [@chaliy](https://github.com/chaliy)

## [0.17.18] - 2026-07-31

### Highlights

- **MCP 2026-07-28 specification** - Adopts the final MCP 2026-07-28 specification ([#2906](https://github.com/everruns/everruns/pull/2906)).
- **Security and reliability fixes** - Throttles chat session creation, enforces session rate limits in commands, bounds file grep and user-preference resource usage, and hardens citation, harness-dispatch, and snapshot handling ([#2902](https://github.com/everruns/everruns/pull/2902), [#2899](https://github.com/everruns/everruns/pull/2899), [#2895](https://github.com/everruns/everruns/pull/2895), [#2898](https://github.com/everruns/everruns/pull/2898), [#2893](https://github.com/everruns/everruns/pull/2893), [#2894](https://github.com/everruns/everruns/pull/2894), [#2900](https://github.com/everruns/everruns/pull/2900)).

### What's Changed

- fix(deps): force @hono/node-server >=2.0.5 in .deepsec (GHSA-frvp-7c67-39w9) ([#2909](https://github.com/everruns/everruns/pull/2909)) by [@chaliy](https://github.com/chaliy)
- fix(deps): patch critical/high npm security advisories (astro, node-tar) ([#2908](https://github.com/everruns/everruns/pull/2908)) by [@chaliy](https://github.com/chaliy)
- chore(deps): upgrade jsonwebtoken 10.4.0 to 11.0.0 ([#2907](https://github.com/everruns/everruns/pull/2907)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): adopt the final MCP 2026-07-28 specification ([#2906](https://github.com/everruns/everruns/pull/2906)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump the cargo group with 7 updates ([#2905](https://github.com/everruns/everruns/pull/2905)) by [@app/dependabot](https://github.com/apps/dependabot)
- chore(deps): bump @hono/node-server from 1.19.15 to 1.19.17 in /.deepsec in the npm_and_yarn group across 1 directory ([#2904](https://github.com/everruns/everruns/pull/2904)) by [@app/dependabot](https://github.com/apps/dependabot)
- chore(deps): bump bashkit to 0.14.4 and fix touch in the session filesystem ([#2903](https://github.com/everruns/everruns/pull/2903)) by [@chaliy](https://github.com/chaliy)
- fix(security): throttle chat session creation ([#2902](https://github.com/everruns/everruns/pull/2902)) by [@chaliy](https://github.com/chaliy)
- fix(files): scope glob grep content scans ([#2896](https://github.com/everruns/everruns/pull/2896)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve filtered history with prompt hooks ([#2891](https://github.com/everruns/everruns/pull/2891)) by [@chaliy](https://github.com/chaliy)
- fix(api): bound user preference storage ([#2898](https://github.com/everruns/everruns/pull/2898)) by [@chaliy](https://github.com/chaliy)
- fix(agent-triggers): preserve harness during dispatch ([#2894](https://github.com/everruns/everruns/pull/2894)) by [@chaliy](https://github.com/chaliy)
- fix(security): enforce session rate limits in commands ([#2899](https://github.com/everruns/everruns/pull/2899)) by [@chaliy](https://github.com/chaliy)
- fix(messages): route turns with public agent IDs ([#2901](https://github.com/everruns/everruns/pull/2901)) by [@chaliy](https://github.com/chaliy)
- fix(citations): guard citation metadata output ([#2893](https://github.com/everruns/everruns/pull/2893)) by [@chaliy](https://github.com/chaliy)
- fix(compaction): preserve stateful request delta ([#2892](https://github.com/everruns/everruns/pull/2892)) by [@chaliy](https://github.com/chaliy)
- fix(local): preserve task webhook secrets in snapshots ([#2900](https://github.com/everruns/everruns/pull/2900)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): bound file grep resource usage ([#2895](https://github.com/everruns/everruns/pull/2895)) by [@chaliy](https://github.com/chaliy)

## [0.17.17] - 2026-07-25

### Highlights

- **Claude Opus 5** - Adds model support and makes it the platform default ([#2885](https://github.com/everruns/everruns/pull/2885)).
- **Tool narration** - Adds narration for session and built-in tool activity ([#2878](https://github.com/everruns/everruns/pull/2878)).
- **Security and reliability fixes** - Hardens prompt rewriting, MCP OAuth grants, provider bindings, secret redaction, and signup URLs ([#2876](https://github.com/everruns/everruns/pull/2876), [#2877](https://github.com/everruns/everruns/pull/2877), [#2879](https://github.com/everruns/everruns/pull/2879), [#2880](https://github.com/everruns/everruns/pull/2880), [#2882](https://github.com/everruns/everruns/pull/2882)).
- **Citations by default** - Enables citations in the generic harness ([#2865](https://github.com/everruns/everruns/pull/2865)).

### What's Changed

- chore(deps): bump fast-uri to 3.1.4 in apps/docs ([#2889](https://github.com/everruns/everruns/pull/2889)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump vulnerable ui transitive deps ([#2888](https://github.com/everruns/everruns/pull/2888)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove duplicated implementation detail ([#2887](https://github.com/everruns/everruns/pull/2887)) by [@chaliy](https://github.com/chaliy)
- chore(agents): apply Claude 5 context engineering to agent docs ([#2886](https://github.com/everruns/everruns/pull/2886)) by [@chaliy](https://github.com/chaliy)
- feat(models): add Claude Opus 5 and make it the platform default ([#2885](https://github.com/everruns/everruns/pull/2885)) by [@chaliy](https://github.com/chaliy)
- feat(core): add tool narration for session and built-in tools ([#2878](https://github.com/everruns/everruns/pull/2878)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): prevent prompt rewrite history bypass ([#2876](https://github.com/everruns/everruns/pull/2876)) by [@chaliy](https://github.com/chaliy)
- fix(evals): redact structured dataset secrets ([#2880](https://github.com/everruns/everruns/pull/2880)) by [@chaliy](https://github.com/chaliy)
- fix(server): bind MCP OAuth grants to provider authority ([#2877](https://github.com/everruns/everruns/pull/2877)) by [@chaliy](https://github.com/chaliy)
- fix(session-tasks): preserve root session on updates ([#2884](https://github.com/everruns/everruns/pull/2884)) by [@chaliy](https://github.com/chaliy)
- fix(providers): reject disabled service bindings ([#2879](https://github.com/everruns/everruns/pull/2879)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve literal workspace display paths ([#2883](https://github.com/everruns/everruns/pull/2883)) by [@chaliy](https://github.com/chaliy)
- fix(auth): keep signup email out of urls ([#2882](https://github.com/everruns/everruns/pull/2882)) by [@chaliy](https://github.com/chaliy)
- fix(cli): truncate table cells on UTF-8 boundaries ([#2881](https://github.com/everruns/everruns/pull/2881)) by [@chaliy](https://github.com/chaliy)
- chore(deps): upgrade docs and security tooling ([`80a0085`](https://github.com/everruns/everruns/commit/80a0085e53a8a9c26090481506d5bce703f3cccd))
- chore(deps): bump next from 16.2.10 to 16.2.11 in apps/ui ([#2874](https://github.com/everruns/everruns/pull/2874)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump the npm_and_yarn group across 1 directory with 2 updates ([#2871](https://github.com/everruns/everruns/pull/2871)) by [@app/dependabot](https://github.com/apps/dependabot)
- feat(harness): enable citations by default in the generic harness ([#2865](https://github.com/everruns/everruns/pull/2865)) by [@chaliy](https://github.com/chaliy)


<!-- New changes go here. Use `/prepare-release X.Y.Z` to generate draft from commits. -->

## [0.17.16] - 2026-07-22

### Highlights

- **Automatic session titles** — Sessions now generate titles automatically via title events, with a policy enforcing consistent automatic titling ([#2867](https://github.com/everruns/everruns/pull/2867), [#2869](https://github.com/everruns/everruns/pull/2869)).
- **Durable context checkpoints** — Compaction persists durable context checkpoints, improving reliability of long-running sessions across compaction ([#2868](https://github.com/everruns/everruns/pull/2868), [#2870](https://github.com/everruns/everruns/pull/2870), [#2866](https://github.com/everruns/everruns/pull/2866)).

### What's Changed

- feat(runtime): surface structured turn stop reasons ([#2864](https://github.com/everruns/everruns/pull/2864)) by [@chaliy](https://github.com/chaliy)
- feat(session): add automatic title events ([#2867](https://github.com/everruns/everruns/pull/2867)) by [@chaliy](https://github.com/chaliy)
- feat(compaction): persist durable context checkpoints ([#2868](https://github.com/everruns/everruns/pull/2868)) by [@chaliy](https://github.com/chaliy)
- fix(compaction): preserve native compact context ([#2866](https://github.com/everruns/everruns/pull/2866)) by [@chaliy](https://github.com/chaliy)
- fix(session): enforce automatic title policy ([#2869](https://github.com/everruns/everruns/pull/2869)) by [@chaliy](https://github.com/chaliy)
- fix(compaction): persist effective proactive checkpoints ([#2870](https://github.com/everruns/everruns/pull/2870)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump the cargo group with 6 updates ([#2863](https://github.com/everruns/everruns/pull/2863))
- chore(deps): bump the npm_and_yarn group across 1 directory with 2 updates ([#2856](https://github.com/everruns/everruns/pull/2856))
- chore(deps): bump rust to 1.97.1-slim across server, worker, and docker ([#2858](https://github.com/everruns/everruns/pull/2858), [#2860](https://github.com/everruns/everruns/pull/2860), [#2862](https://github.com/everruns/everruns/pull/2862))
- chore(deps): bump distroless/cc-debian12 across server, worker, and docker ([#2857](https://github.com/everruns/everruns/pull/2857), [#2859](https://github.com/everruns/everruns/pull/2859), [#2861](https://github.com/everruns/everruns/pull/2861))

## [0.17.15] - 2026-07-20

### Highlights

- **Claim-level citations** — Added composable citation capabilities so agents can attach sources directly to individual claims ([#2852](https://github.com/everruns/everruns/pull/2852)).

### What's Changed

- feat(citations): claim-level citations as composable capabilities ([#2852](https://github.com/everruns/everruns/pull/2852)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): validate tool context services ([#2854](https://github.com/everruns/everruns/pull/2854)) by [@chaliy](https://github.com/chaliy)
- style(docs): refine sidebar hierarchy ([#2853](https://github.com/everruns/everruns/pull/2853)) by [@chaliy](https://github.com/chaliy)
- test(ui): browser regression test for Settings org create dialog ([#2851](https://github.com/everruns/everruns/pull/2851)) by [@chaliy](https://github.com/chaliy)

## [0.17.14] - 2026-07-19

### Highlights

- **Kimi K3 model** — Added a model profile for Kimi K3, so it can be selected as a provider model ([#2848](https://github.com/everruns/everruns/pull/2848)).

### What's Changed

- feat(provider): add Kimi K3 model profile ([#2848](https://github.com/everruns/everruns/pull/2848)) by [@chaliy](https://github.com/chaliy)
- refactor(core): avoid if_let_guard in spawn-mode parsing ([#2847](https://github.com/everruns/everruns/pull/2847)) by [@chaliy](https://github.com/chaliy)

## [0.17.13] - 2026-07-19

### Highlights

- **Agent-first CLI** — New `triggers` and `participants` CLI commands, and agent-trigger sessions are now owned by the agent's own identity ([#2828](https://github.com/everruns/everruns/pull/2828), [#2829](https://github.com/everruns/everruns/pull/2829)).
- **Steadier live streaming** — Streaming message lifecycle events are correlated and output is projected by message id, so in-progress responses render more reliably ([#2842](https://github.com/everruns/everruns/pull/2842), [#2843](https://github.com/everruns/everruns/pull/2843)).

### What's Changed

- feat(runtime): reconfigure live capabilities ([#2844](https://github.com/everruns/everruns/pull/2844)) by [@chaliy](https://github.com/chaliy)
- feat(events): correlate streaming message lifecycle ([#2842](https://github.com/everruns/everruns/pull/2842)) by [@chaliy](https://github.com/chaliy)
- feat(agents): own agent-trigger sessions by the agent's identity ([#2829](https://github.com/everruns/everruns/pull/2829)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add triggers and participants commands ([#2828](https://github.com/everruns/everruns/pull/2828)) by [@chaliy](https://github.com/chaliy)
- fix(streaming): project output by message id ([#2843](https://github.com/everruns/everruns/pull/2843)) by [@chaliy](https://github.com/chaliy)
- fix(server): refresh expiring MCP OAuth tokens ([#2841](https://github.com/everruns/everruns/pull/2841)) by [@chaliy](https://github.com/chaliy)
- fix(core): keep compacted tool exchanges atomic ([#2840](https://github.com/everruns/everruns/pull/2840)) by [@chaliy](https://github.com/chaliy)
- fix(agent-triggers): add execution-context columns to preserve app context ([#2832](https://github.com/everruns/everruns/pull/2832)) by [@chaliy](https://github.com/chaliy)
- fix(agent-identities): block archived trigger owners ([#2831](https://github.com/everruns/everruns/pull/2831)) by [@chaliy](https://github.com/chaliy)
- fix(export): enforce session view on exports ([#2836](https://github.com/everruns/everruns/pull/2836)) by [@chaliy](https://github.com/chaliy)
- fix(server): gate AG-UI reasoning summaries ([#2835](https://github.com/everruns/everruns/pull/2835)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): validate package identity on update ([#2834](https://github.com/everruns/everruns/pull/2834)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): enforce endpoint gate on org overrides ([#2833](https://github.com/everruns/everruns/pull/2833)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): fall back on unsupported RC versions by [@chaliy](https://github.com/chaliy)
- fix(runtime): forward contextual grep through decorators ([#2830](https://github.com/everruns/everruns/pull/2830)) by [@chaliy](https://github.com/chaliy)
- perf(ui): eliminate Early Access banner layout shift ([#2839](https://github.com/everruns/everruns/pull/2839)) by [@chaliy](https://github.com/chaliy)
- perf(ui): stop /agents/new prefetches on Agents page startup ([#2838](https://github.com/everruns/everruns/pull/2838)) by [@chaliy](https://github.com/chaliy)
- refactor: extract everruns-provider so providers don't depend on core ([#2825](https://github.com/everruns/everruns/pull/2825)) by [@chaliy](https://github.com/chaliy)
- refactor(ard): move ARD from integrations to a platform crate ([#2824](https://github.com/everruns/everruns/pull/2824)) by [@chaliy](https://github.com/chaliy)

## [0.17.12] - 2026-07-17

### What's Changed

- feat(evals): generic in-process eval study over the local runtime ([#2818](https://github.com/everruns/everruns/pull/2818)) by [@chaliy](https://github.com/chaliy)
- fix(core): keep embedder display policy in system-prompt file store ([#2819](https://github.com/everruns/everruns/pull/2819)) by [@chaliy](https://github.com/chaliy)

## [0.17.11] - 2026-07-16

### What's Changed

- feat(core): MountFs display policy seam for host-native paths ([#2816](https://github.com/everruns/everruns/pull/2816)) by [@chaliy](https://github.com/chaliy)
- fix(ci): run workflow tests against real gRPC control-plane ([#2799](https://github.com/everruns/everruns/pull/2799)) by [@chaliy](https://github.com/chaliy)

## [0.17.10] - 2026-07-16

### Highlights

- **Expanded global search** — Global search now covers more of the workspace, so results surface across more session and content types ([#2810](https://github.com/everruns/everruns/pull/2810)).
- **Resend email plugin & plugin MCP OAuth** — New Resend OAuth email plugin, plus OAuth support for plugin-provided MCP servers ([#2808](https://github.com/everruns/everruns/pull/2808)).
- **Web fetch on fetchkit 0.5** — The web-fetch tool adopts fetchkit 0.5 capabilities ([#2807](https://github.com/everruns/everruns/pull/2807)).
- **Live reasoning stream** — Streamed phase hints on message start/delta and projected `reason.item` summaries into the reasoning channel make in-progress model reasoning more visible ([#2804](https://github.com/everruns/everruns/pull/2804), [#2803](https://github.com/everruns/everruns/pull/2803)).

### What's Changed

- feat(ui): expand global search coverage ([#2810](https://github.com/everruns/everruns/pull/2810)) by [@chaliy](https://github.com/chaliy)
- feat(plugins): Resend OAuth email plugin + plugin MCP OAuth support ([#2808](https://github.com/everruns/everruns/pull/2808)) by [@chaliy](https://github.com/chaliy)
- feat(web-fetch): adopt fetchkit 0.5 features ([#2807](https://github.com/everruns/everruns/pull/2807)) by [@chaliy](https://github.com/chaliy)
- feat(core): streamed phase hint on output.message started/delta ([#2804](https://github.com/everruns/everruns/pull/2804)) by [@chaliy](https://github.com/chaliy)
- feat(server): project reason.item summaries to reasoning channel ([#2803](https://github.com/everruns/everruns/pull/2803)) by [@chaliy](https://github.com/chaliy)
- feat(filesystem): return bounded grep context by [@chaliy](https://github.com/chaliy)
- perf(api): compress non-streaming responses ([#2811](https://github.com/everruns/everruns/pull/2811)) by [@chaliy](https://github.com/chaliy)
- perf(ui): scope sidebar prefetch to intent ([#2809](https://github.com/everruns/everruns/pull/2809)) by [@chaliy](https://github.com/chaliy)
- perf(agent): teach shared hints single-read/contextual-search policy ([#2805](https://github.com/everruns/everruns/pull/2805)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): keep grep available by redacting private memory ([#2801](https://github.com/everruns/everruns/pull/2801)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): honor grep files regex contract ([#2800](https://github.com/everruns/everruns/pull/2800)) by [@chaliy](https://github.com/chaliy)
- fix(agents): restore harness on version rollback ([#2797](https://github.com/everruns/everruns/pull/2797)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): guard report_result task finalization ([#2796](https://github.com/everruns/everruns/pull/2796)) by [@chaliy](https://github.com/chaliy)
- fix(providers): validate KB embedding provider service ([#2795](https://github.com/everruns/everruns/pull/2795)) by [@chaliy](https://github.com/chaliy)
- fix(session-tasks): restrict scheduled probe tools ([#2794](https://github.com/everruns/everruns/pull/2794)) by [@chaliy](https://github.com/chaliy)
- fix(harnesses): bound embedder metadata ([#2793](https://github.com/everruns/everruns/pull/2793)) by [@chaliy](https://github.com/chaliy)
- fix(core): reject malformed legacy edit scalars ([#2792](https://github.com/everruns/everruns/pull/2792)) by [@chaliy](https://github.com/chaliy)
- fix(core): preserve accumulator effective cost ([#2791](https://github.com/everruns/everruns/pull/2791)) by [@chaliy](https://github.com/chaliy)
- fix(server): reconcile canceled one-shot monitors ([#2790](https://github.com/everruns/everruns/pull/2790)) by [@chaliy](https://github.com/chaliy)
- test(durable): make counter reset atomic ([#2789](https://github.com/everruns/everruns/pull/2789)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): require tasks opt-in for task methods ([#2788](https://github.com/everruns/everruns/pull/2788)) by [@chaliy](https://github.com/chaliy)
- fix(core): require spawn_agent target fields ([#2787](https://github.com/everruns/everruns/pull/2787)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): seed participant agent versions ([#2786](https://github.com/everruns/everruns/pull/2786)) by [@chaliy](https://github.com/chaliy)
- fix(reporting): guard malformed usage records ([#2785](https://github.com/everruns/everruns/pull/2785)) by [@chaliy](https://github.com/chaliy)
- fix(auth): clear signup return_to after instant auth ([#2784](https://github.com/everruns/everruns/pull/2784)) by [@chaliy](https://github.com/chaliy)
- fix(llm): enforce model speed tier support ([#2783](https://github.com/everruns/everruns/pull/2783)) by [@chaliy](https://github.com/chaliy)
- fix(worker): preserve non-wake durable signals ([#2782](https://github.com/everruns/everruns/pull/2782)) by [@chaliy](https://github.com/chaliy)
- fix(core): accept legacy A2A wait run records ([#2781](https://github.com/everruns/everruns/pull/2781)) by [@chaliy](https://github.com/chaliy)
- fix(server): remove org feature flag read cache ([#2780](https://github.com/everruns/everruns/pull/2780)) by [@chaliy](https://github.com/chaliy)
- fix(evals): validate imported attribution urls ([#2777](https://github.com/everruns/everruns/pull/2777)) by [@chaliy](https://github.com/chaliy)
- fix(fs): keep mounted display paths host-agnostic ([#2776](https://github.com/everruns/everruns/pull/2776)) by [@chaliy](https://github.com/chaliy)
- fix(providers): block unsafe trace links ([#2775](https://github.com/everruns/everruns/pull/2775)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): restore endpoint feature gate ([#2774](https://github.com/everruns/everruns/pull/2774)) by [@chaliy](https://github.com/chaliy)
- fix(ui): pin design lint dependency ([#2773](https://github.com/everruns/everruns/pull/2773)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): reject duplicate routing headers ([#2772](https://github.com/everruns/everruns/pull/2772)) by [@chaliy](https://github.com/chaliy)
- fix(storage): avoid deleting reusable file blob keys ([#2771](https://github.com/everruns/everruns/pull/2771)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): scope HTTP negotiation cache ([#2770](https://github.com/everruns/everruns/pull/2770)) by [@chaliy](https://github.com/chaliy)
- fix(ui): bound eval run comparison inputs ([#2768](https://github.com/everruns/everruns/pull/2768)) by [@chaliy](https://github.com/chaliy)
- fix(core): prevent host-scope symlink escapes ([#2767](https://github.com/everruns/everruns/pull/2767)) by [@chaliy](https://github.com/chaliy)
- fix(public-chat): bind sessions to visitor identity ([#2766](https://github.com/everruns/everruns/pull/2766)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): cap background watcher concurrency ([#2765](https://github.com/everruns/everruns/pull/2765)) by [@chaliy](https://github.com/chaliy)
- fix(apps): reject future cron bursts ([#2764](https://github.com/everruns/everruns/pull/2764)) by [@chaliy](https://github.com/chaliy)
- fix(bashkit): stream capped HTTP responses ([#2763](https://github.com/everruns/everruns/pull/2763)) by [@chaliy](https://github.com/chaliy)
- fix(drivers): bound streaming reconnect first item wait ([#2762](https://github.com/everruns/everruns/pull/2762)) by [@chaliy](https://github.com/chaliy)
- fix(core): fence subagent progress attempts ([#2761](https://github.com/everruns/everruns/pull/2761)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): gate session overrides as high risk ([#2760](https://github.com/everruns/everruns/pull/2760)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): serialize spawn admission ([#2759](https://github.com/everruns/everruns/pull/2759)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): require agent permission for agent sessions ([#2757](https://github.com/everruns/everruns/pull/2757)) by [@chaliy](https://github.com/chaliy)
- fix(models): apply tiered cost estimates ([#2755](https://github.com/everruns/everruns/pull/2755)) by [@chaliy](https://github.com/chaliy)
- fix(core): keep spawn_agent schema free of top-level oneOf ([#2806](https://github.com/everruns/everruns/pull/2806)) by [@chaliy](https://github.com/chaliy)
- fix(core): preserve workspace display paths ([#2779](https://github.com/everruns/everruns/pull/2779)) by [@chaliy](https://github.com/chaliy)
- fix(auth): harden recovery endpoint timing ([#2769](https://github.com/everruns/everruns/pull/2769)) by [@chaliy](https://github.com/chaliy)
- perf(ui): avoid dashboard capability and model overfetch ([#2814](https://github.com/everruns/everruns/pull/2814)) by [@chaliy](https://github.com/chaliy)
- perf(ui): eliminate duplicate dashboard agent-list requests ([#2813](https://github.com/everruns/everruns/pull/2813)) by [@chaliy](https://github.com/chaliy)
- perf(ui): stop automatic /agents/new prefetch on dashboard startup ([#2815](https://github.com/everruns/everruns/pull/2815)) by [@chaliy](https://github.com/chaliy)

## [0.17.9] - 2026-07-14

### Highlights

- **Participant rail & session addressing** — Sessions now show a live participant rail with join/leave lines and explicit addressing, making multi-agent conversations easier to follow ([#2718](https://github.com/everruns/everruns/pull/2718)).
- **Cross-session Work view** — A new Work view groups tasks across sessions by their delegation tree, so you can see delegated work in one place ([#2715](https://github.com/everruns/everruns/pull/2715)).
- **Expanded model catalog** — Surfaced Sonnet 5 and recent Gemini models, defaulted to GPT-5.6 Sol, and seeded Claude Opus 4.8 ([#2712](https://github.com/everruns/everruns/pull/2712), [#2713](https://github.com/everruns/everruns/pull/2713)).
- **Per-message verbosity control** — GPT-5.x sessions can now set output verbosity per message ([#2717](https://github.com/everruns/everruns/pull/2717)).
- **Resilient usage limits** — Sessions auto-continue after LLM usage-limit errors instead of stalling ([#2714](https://github.com/everruns/everruns/pull/2714)).

### What's Changed

- feat(agents): unify structured delegation results ([#2722](https://github.com/everruns/everruns/pull/2722)) by [@chaliy](https://github.com/chaliy)
- feat(agents): agent_triggers data model + storage (EVE-757 A) ([#2721](https://github.com/everruns/everruns/pull/2721)) by [@chaliy](https://github.com/chaliy)
- fix(local): scope schedule claims to routable sessions ([#2720](https://github.com/everruns/everruns/pull/2720)) by [@chaliy](https://github.com/chaliy)
- feat(ui): participant rail, join/leave lines, and addressing (EVE-759) ([#2718](https://github.com/everruns/everruns/pull/2718)) by [@chaliy](https://github.com/chaliy)
- feat(models): add per-message verbosity control for GPT-5.x ([#2717](https://github.com/everruns/everruns/pull/2717)) by [@chaliy](https://github.com/chaliy)
- fix(examples): add harness_id to weekend-concierge Agent initializer ([#2716](https://github.com/everruns/everruns/pull/2716)) by [@chaliy](https://github.com/chaliy)
- feat(ui): cross-session Work view grouping tasks by delegation tree ([#2715](https://github.com/everruns/everruns/pull/2715)) by [@chaliy](https://github.com/chaliy)
- feat(core): auto-continue after LLM usage-limit errors ([#2714](https://github.com/everruns/everruns/pull/2714)) by [@chaliy](https://github.com/chaliy)
- chore(models): default to GPT-5.6 Sol and seed Claude Opus 4.8 ([#2713](https://github.com/everruns/everruns/pull/2713)) by [@chaliy](https://github.com/chaliy)
- feat(models): surface Sonnet 5 and recent Gemini models in catalog ([#2712](https://github.com/everruns/everruns/pull/2712)) by [@chaliy](https://github.com/chaliy)
- fix(files): support glob path filters in grep ([#2707](https://github.com/everruns/everruns/pull/2707)) by [@chaliy](https://github.com/chaliy)
- fix(tools): preserve requested output verbosity ([#2705](https://github.com/everruns/everruns/pull/2705)) by [@chaliy](https://github.com/chaliy)

## [0.17.8] - 2026-07-11

### Highlights

- **Embedded schedule execution** — `everruns-local` now runs due one-shot and recurring session schedules through the host's live `LocalSessionRunner`, with atomic claims, stale recovery, retryable delivery, timezone-aware advancement, and graceful lifecycle. Embedded hosts that enable scheduled monitors must start and retain `LocalScheduleRunner` ([#2706](https://github.com/everruns/everruns/pull/2706)).

### What's Changed

- feat(local): execute due session schedules ([#2706](https://github.com/everruns/everruns/pull/2706)) by [@chaliy](https://github.com/chaliy)
- fix(auth): harden audit client IP extraction ([#2704](https://github.com/everruns/everruns/pull/2704)) by [@chaliy](https://github.com/chaliy)
- fix(llm): retry structured in-band provider errors ([#2702](https://github.com/everruns/everruns/pull/2702)) by [@chaliy](https://github.com/chaliy)
- test(local): stabilize spawn_agent runtime proof ([#2703](https://github.com/everruns/everruns/pull/2703)) by [@chaliy](https://github.com/chaliy)

## [0.17.7] - 2026-07-11

### Highlights

- **Real-time session tasks** — Task state transitions now wake a running session mid-turn on the durable path, deliver every transition, and support per-task webhook configs and structured task results over A2A + MCP (EVE-681) ([#2699](https://github.com/everruns/everruns/pull/2699), [#2691](https://github.com/everruns/everruns/pull/2691), [#2690](https://github.com/everruns/everruns/pull/2690), [#2681](https://github.com/everruns/everruns/pull/2681), [#2674](https://github.com/everruns/everruns/pull/2674)).
- **Scoped agent & user memory** — Added scoped memory for agents and users.
- **Composer model/effort menu** — Combined model and effort selection into one composer menu with recent models ([#2663](https://github.com/everruns/everruns/pull/2663)).
- **Speed selector** — New LLM speed selector mapped to OpenAI's `service_tier` ([#2669](https://github.com/everruns/everruns/pull/2669)).
- **Detached & handoff sessions** — Spawn detached peer sessions and invite handoff agents ([#2684](https://github.com/everruns/everruns/pull/2684), [#2655](https://github.com/everruns/everruns/pull/2655)).

### What's Changed

- feat(session-tasks): deliver task wakes mid-turn on the durable path (EVE-681) ([#2699](https://github.com/everruns/everruns/pull/2699)) by [@chaliy](https://github.com/chaliy)
- fix(test): stop spawn_agent llmsim proof double-spawning a subagent ([#2700](https://github.com/everruns/everruns/pull/2700)) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): surface root_session_id on task reads (EVE-681) ([#2690](https://github.com/everruns/everruns/pull/2690)) by [@chaliy](https://github.com/chaliy)
- docs(api): use scanner-safe example for push-config secret ([#2696](https://github.com/everruns/everruns/pull/2696)) by [@chaliy](https://github.com/chaliy)
- docs(mcp): reconcile tasks field names by [@chaliy](https://github.com/chaliy)
- fix(duckduckgo): clarify instant-answer scope, add empty-result caveat ([#2689](https://github.com/everruns/everruns/pull/2689)) by [@chaliy](https://github.com/chaliy)
- fix(core): make activate_skill missing-name error self-repairing ([#2694](https://github.com/everruns/everruns/pull/2694)) by [@chaliy](https://github.com/chaliy)
- chore(process-issues): doubt issue text and reproduce before fixing ([#2695](https://github.com/everruns/everruns/pull/2695)) by [@chaliy](https://github.com/chaliy)
- feat(export): export image content and link subagent trajectories in ATIF ([#2692](https://github.com/everruns/everruns/pull/2692)) by [@chaliy](https://github.com/chaliy)
- test(delegation): in-process spawn_agent runtime proof via llmsim ([#2687](https://github.com/everruns/everruns/pull/2687)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add detached peer session spawning ([#2684](https://github.com/everruns/everruns/pull/2684)) by [@chaliy](https://github.com/chaliy)
- feat(runtime): mid-turn task wake delivery — EVE-681 (part A) ([#2691](https://github.com/everruns/everruns/pull/2691)) by [@chaliy](https://github.com/chaliy)
- fix(durable): make workflow start idempotent ([#2688](https://github.com/everruns/everruns/pull/2688)) by [@chaliy](https://github.com/chaliy)
- feat(core): embeddable in-process task transition observer (EVE-729) ([#2686](https://github.com/everruns/everruns/pull/2686)) by [@chaliy](https://github.com/chaliy)
- test(cli): wait for harness seed before agent e2e ([#2685](https://github.com/everruns/everruns/pull/2685)) by [@chaliy](https://github.com/chaliy)
- fix(storage): bound message history reads ([#2682](https://github.com/everruns/everruns/pull/2682)) by [@chaliy](https://github.com/chaliy)
- feat(api): surface structured task result via A2A + MCP ([#2681](https://github.com/everruns/everruns/pull/2681)) by [@chaliy](https://github.com/chaliy)
- fix(reporting): optimize fact_session projection for large histories ([#2680](https://github.com/everruns/everruns/pull/2680)) by [@chaliy](https://github.com/chaliy)
- test(memory): expect scoped memory in workflow fs test ([#2679](https://github.com/everruns/everruns/pull/2679)) by [@chaliy](https://github.com/chaliy)
- feat(cli): typed --harness flag on agents create/update (EVE-693) by [@chaliy](https://github.com/chaliy)
- feat(memory): add scoped agent and user memory by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): per-task webhook configs + all-transition delivery ([#2674](https://github.com/everruns/everruns/pull/2674)) by [@chaliy](https://github.com/chaliy)
- test(catalog): refresh agent MCP and app cases ([#2677](https://github.com/everruns/everruns/pull/2677)) by [@chaliy](https://github.com/chaliy)
- test(sessions): refresh API contract cases ([#2676](https://github.com/everruns/everruns/pull/2676)) by [@chaliy](https://github.com/chaliy)
- fix(fs): present persisted output paths via file store ([#2673](https://github.com/everruns/everruns/pull/2673)) by [@chaliy](https://github.com/chaliy)
- fix(signup): show inline password-policy error instead of native block ([#2672](https://github.com/everruns/everruns/pull/2672)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): rename interim tool to report_task_progress ([#2671](https://github.com/everruns/everruns/pull/2671)) by [@chaliy](https://github.com/chaliy)
- feat(llm): add speed selector mapped to OpenAI service_tier ([#2669](https://github.com/everruns/everruns/pull/2669)) by [@chaliy](https://github.com/chaliy)
- fix(auth): return fresh user name after profile updates ([#2668](https://github.com/everruns/everruns/pull/2668)) by [@chaliy](https://github.com/chaliy)
- feat(ui): combined model/effort composer menu with recent models ([#2663](https://github.com/everruns/everruns/pull/2663)) by [@chaliy](https://github.com/chaliy)
- fix(fs): preserve primary mount path identity ([#2666](https://github.com/everruns/everruns/pull/2666)) by [@chaliy](https://github.com/chaliy)
- feat(mcp-ui): edit MCP server name, description, and URL ([#2670](https://github.com/everruns/everruns/pull/2670)) by [@chaliy](https://github.com/chaliy)
- fix(auth): honor GitHub-reported email_verified in OAuth ([#2656](https://github.com/everruns/everruns/pull/2656)) by [@chaliy](https://github.com/chaliy)
- fix(core): strip echoed [time] annotations from assistant replies ([#2657](https://github.com/everruns/everruns/pull/2657)) by [@chaliy](https://github.com/chaliy)
- fix(members-ui): hide owner-only controls from org admins, show errors ([#2667](https://github.com/everruns/everruns/pull/2667)) by [@chaliy](https://github.com/chaliy)
- docs(skills): require PR template structure by [@chaliy](https://github.com/chaliy)
- fix(test-cases): align TC010 disabled-chat copy with Platform Chat rebrand ([#2658](https://github.com/everruns/everruns/pull/2658)) by [@chaliy](https://github.com/chaliy)
- chore(policy): allow yolop attribution by [@chaliy](https://github.com/chaliy)
- fix(auth): normalize user email to case-insensitive identity ([#2661](https://github.com/everruns/everruns/pull/2661)) by [@chaliy](https://github.com/chaliy)
- fix(auth): derive JWT platform roles from database ([#2659](https://github.com/everruns/everruns/pull/2659)) by [@chaliy](https://github.com/chaliy)
- docs(skills): clarify issue processing workflow by [@chaliy](https://github.com/chaliy)
- feat(sessions): invite handoff agents ([#2655](https://github.com/everruns/everruns/pull/2655)) by [@chaliy](https://github.com/chaliy)
- fix(auth): preserve invite return_to through signup ([#2654](https://github.com/everruns/everruns/pull/2654)) by [@chaliy](https://github.com/chaliy)
- fix(auth): derive rate-limit client IP from trusted proxy hops, not leftmost XFF ([#2652](https://github.com/everruns/everruns/pull/2652)) by [@chaliy](https://github.com/chaliy)
- fix(auth): reject unsupported personal access token scopes at creation ([#2651](https://github.com/everruns/everruns/pull/2651)) by [@chaliy](https://github.com/chaliy)
- fix(auth-ui): refetch current user after email verification ([#2653](https://github.com/everruns/everruns/pull/2653)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): settle session to idle when a turn is cancelled ([#2650](https://github.com/everruns/everruns/pull/2650)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): reject harnesses requiring unavailable capabilities ([#2649](https://github.com/everruns/everruns/pull/2649)) by [@chaliy](https://github.com/chaliy)
- chore(skills): remove agent-browser skill ([#2648](https://github.com/everruns/everruns/pull/2648)) by [@chaliy](https://github.com/chaliy)
- chore(cursor): remove cloud agent environment config ([#2642](https://github.com/everruns/everruns/pull/2642)) by [@chaliy](https://github.com/chaliy)

## [0.17.6] - 2026-07-10

### Highlights

- **ATIF session/eval export** — Export sessions and eval datasets to ATIF, segmented for large sessions with image refs, size guards, and CLI (`--format atif`) support ([#2591](https://github.com/everruns/everruns/pull/2591), [#2609](https://github.com/everruns/everruns/pull/2609), [#2610](https://github.com/everruns/everruns/pull/2610), [#2619](https://github.com/everruns/everruns/pull/2619), [#2626](https://github.com/everruns/everruns/pull/2626), [#2627](https://github.com/everruns/everruns/pull/2627), [#2596](https://github.com/everruns/everruns/pull/2596)).
- **New model profiles** — Added GPT-5.6 Sol, Terra, and Luna ([#2631](https://github.com/everruns/everruns/pull/2631)).
- **Streaming reliability** — LLM streaming responses now reconnect automatically on transport failure ([#2594](https://github.com/everruns/everruns/pull/2594)).
- **Auth fixes** — Closed reachability dead-ends across the auth flow, added OAuth password-reset links, and improved the signup path ([#2606](https://github.com/everruns/everruns/pull/2606), [#2603](https://github.com/everruns/everruns/pull/2603), [#2592](https://github.com/everruns/everruns/pull/2592)).

### What's Changed

- feat(email): add logo to branded template by [@chaliy](https://github.com/chaliy)
- feat(sessions): record user participant provenance ([#2639](https://github.com/everruns/everruns/pull/2639)) by [@chaliy](https://github.com/chaliy)
- perf(sessions): add org created-at index (EVE-697) by [@chaliy](https://github.com/chaliy)
- perf(sessions): eliminate list N+1 queries (EVE-695) ([#2637](https://github.com/everruns/everruns/pull/2637)) by [@chaliy](https://github.com/chaliy)
- fix(reporting): bound event projection repair (EVE-696) by [@chaliy](https://github.com/chaliy)
- chore: deep maintenance — refresh UI deps, fix specs drift ([#2617](https://github.com/everruns/everruns/pull/2617)) by [@chaliy](https://github.com/chaliy)
- feat(models): add GPT-5.6 Sol, Terra, and Luna profiles ([#2631](https://github.com/everruns/everruns/pull/2631)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): route addressed participant turns ([#2634](https://github.com/everruns/everruns/pull/2634)) by [@chaliy](https://github.com/chaliy)
- perf(ui): reduce Settings prefetches (EVE-694) ([#2635](https://github.com/everruns/everruns/pull/2635)) by [@chaliy](https://github.com/chaliy)
- fix(auth): gate default-org auto-join behind opt-in flag ([#2632](https://github.com/everruns/everruns/pull/2632)) by [@chaliy](https://github.com/chaliy)
- fix(openapi): add sdk model metadata ([#2633](https://github.com/everruns/everruns/pull/2633)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add participant API by [@chaliy](https://github.com/chaliy)
- feat(sessions): agent-first session creation (EVE-686 B–F) by [@chaliy](https://github.com/chaliy)
- feat(ui): segmented ATIF export for large sessions ([#2627](https://github.com/everruns/everruns/pull/2627)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add participant model ([#2629](https://github.com/everruns/everruns/pull/2629)) by [@chaliy](https://github.com/chaliy)
- feat(export): segmented ATIF export for large sessions ([#2626](https://github.com/everruns/everruns/pull/2626)) by [@chaliy](https://github.com/chaliy)
- test(subagents): cover nested reaper orphan handling ([#2625](https://github.com/everruns/everruns/pull/2625)) by [@chaliy](https://github.com/chaliy)
- feat(subagents): cap root task fanout ([#2624](https://github.com/everruns/everruns/pull/2624)) by [@chaliy](https://github.com/chaliy)
- feat(budgets): share root session budgets with subagents ([#2623](https://github.com/everruns/everruns/pull/2623)) by [@chaliy](https://github.com/chaliy)
- test(auth): manual reachability cases for the auth state machine ([#2622](https://github.com/everruns/everruns/pull/2622)) by [@chaliy](https://github.com/chaliy)
- feat(subagents): enforce governed nesting depth ([#2621](https://github.com/everruns/everruns/pull/2621)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): denormalize root_session_id for delegation trees ([#2620](https://github.com/everruns/everruns/pull/2620)) by [@chaliy](https://github.com/chaliy)
- feat(agents): persist agent harness ownership ([#2618](https://github.com/everruns/everruns/pull/2618)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add --format atif to sessions export ([#2619](https://github.com/everruns/everruns/pull/2619)) by [@chaliy](https://github.com/chaliy)
- chore(specs): add auth flow-reachability state diagram ([#2616](https://github.com/everruns/everruns/pull/2616)) by [@chaliy](https://github.com/chaliy)
- feat(core): add subagent progress schema by [@chaliy](https://github.com/chaliy)
- chore(ui): typecheck on the native TypeScript 7 compiler ([#2612](https://github.com/everruns/everruns/pull/2612)) by [@chaliy](https://github.com/chaliy)
- feat(subagents): report schema-bound task results by [@chaliy](https://github.com/chaliy)
- fix(runtime): default to runtime-safe capabilities by [@chaliy](https://github.com/chaliy)
- feat(export): ATIF export image refs and size guard ([#2610](https://github.com/everruns/everruns/pull/2610)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): retire legacy delegation tools by [@chaliy](https://github.com/chaliy)
- feat(ui): ATIF session export with limit alerts ([#2609](https://github.com/everruns/everruns/pull/2609)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): dispatch spawn_agent targets by [@chaliy](https://github.com/chaliy)
- fix(auth): close reachability dead-ends across the auth flow ([#2606](https://github.com/everruns/everruns/pull/2606)) by [@chaliy](https://github.com/chaliy)
- feat(agent-handoff): expose spawn_agent target by [@chaliy](https://github.com/chaliy)
- chore: standardize PR descriptions on functional change + before/after ([#2605](https://github.com/everruns/everruns/pull/2605)) by [@chaliy](https://github.com/chaliy)
- feat(subagents): expose spawn_agent subagent target by [@chaliy](https://github.com/chaliy)
- fix(auth): send reset links for oauth accounts ([#2603](https://github.com/everruns/everruns/pull/2603)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): extend live-turn resilience to the thinking matrix ([#2602](https://github.com/everruns/everruns/pull/2602)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): make live agent-run matrix resilient to transient flakes ([#2601](https://github.com/everruns/everruns/pull/2601)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): retry live tool-call case to absorb model non-determinism ([#2600](https://github.com/everruns/everruns/pull/2600)) by [@chaliy](https://github.com/chaliy)
- feat(handoff): give agent handoffs a dedicated task kind ([#2599](https://github.com/everruns/everruns/pull/2599)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): repin Fireworks live case to served Kimi K2 model ([#2598](https://github.com/everruns/everruns/pull/2598)) by [@chaliy](https://github.com/chaliy)
- feat(evals): add ATIF dataset export and case import ([#2596](https://github.com/everruns/everruns/pull/2596)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): repin Fireworks live case to llama-v3p3-70b ([#2597](https://github.com/everruns/everruns/pull/2597)) by [@chaliy](https://github.com/chaliy)
- fix(egress): keep host transports outside runtime policy ([#2595](https://github.com/everruns/everruns/pull/2595)) by [@chaliy](https://github.com/chaliy)
- feat(drivers): reconnect streaming LLM responses on transport failure ([#2594](https://github.com/everruns/everruns/pull/2594)) by [@chaliy](https://github.com/chaliy)
- feat(export): add ATIF session export ([#2591](https://github.com/everruns/everruns/pull/2591)) by [@chaliy](https://github.com/chaliy)
- fix(auth): signup path from password screen + prefill + copy space ([#2592](https://github.com/everruns/everruns/pull/2592)) by [@chaliy](https://github.com/chaliy)
- test(llm-tests): revert "skip live matrix on transient transport errors" ([#2556](https://github.com/everruns/everruns/pull/2556)) by [@chaliy](https://github.com/chaliy)

## [0.17.5] - 2026-07-08

### Highlights

- **Public Chat** — Turn an App into a standalone, branded public-facing chat site behind a feature flag, with anonymous or Google sign-in and Turnstile bot protection ([#2400](https://github.com/everruns/everruns/pull/2400)).
- **Redesigned sign-up & login** — Explicit signup path with an email-confirmation gate, unified auth entry, and clearer credential-failure recovery ([#2574](https://github.com/everruns/everruns/pull/2574), [#2572](https://github.com/everruns/everruns/pull/2572), [#2571](https://github.com/everruns/everruns/pull/2571), [#2573](https://github.com/everruns/everruns/pull/2573), [#2570](https://github.com/everruns/everruns/pull/2570)).
- **Eval run share links** — Publish read-only, revocable share links for eval runs ([#2565](https://github.com/everruns/everruns/pull/2565)).
- **Background subagents** — `spawn_subagent` now runs in the background by default, waking the parent on completion ([#2576](https://github.com/everruns/everruns/pull/2576)).
- **Claude Sonnet 5 model support** ([#2583](https://github.com/everruns/everruns/pull/2583)).

### What's Changed

- feat(runtime): add multi-root host workspace mounts by [@chaliy](https://github.com/chaliy)
- feat(auth): link OAuth identity to existing account by verified email ([#2570](https://github.com/everruns/everruns/pull/2570)) by [@chaliy](https://github.com/chaliy)
- feat(evals): read-only share links for eval runs ([#2565](https://github.com/everruns/everruns/pull/2565)) by [@chaliy](https://github.com/chaliy)
- fix(server): map MCP/plugin validation errors to 400 ([#2534](https://github.com/everruns/everruns/pull/2534)) by [@chaliy](https://github.com/chaliy)
- feat(public-chat): isolated public chat app behind feature flag ([#2400](https://github.com/everruns/everruns/pull/2400)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unified auth entry and onboarding arc continuity ([#2571](https://github.com/everruns/everruns/pull/2571)) by [@chaliy](https://github.com/chaliy)
- feat(auth): abuse limits, password cap, logout revoke, oauth UX ([#2572](https://github.com/everruns/everruns/pull/2572)) by [@chaliy](https://github.com/chaliy)
- fix(ui): offer password reset from login credential-failure alert ([#2573](https://github.com/everruns/everruns/pull/2573)) by [@chaliy](https://github.com/chaliy)
- feat(auth): explicit signup path with email-confirm gate ([#2574](https://github.com/everruns/everruns/pull/2574)) by [@chaliy](https://github.com/chaliy)
- feat(subagents): background spawn_subagent with foreground opt-in ([#2576](https://github.com/everruns/everruns/pull/2576)) by [@chaliy](https://github.com/chaliy)
- feat(models): add Claude Sonnet 5 model profile ([#2583](https://github.com/everruns/everruns/pull/2583)) by [@chaliy](https://github.com/chaliy)
- fix(auth): require verified email before auto-linking OAuth identity ([#2575](https://github.com/everruns/everruns/pull/2575)) by [@chaliy](https://github.com/chaliy)
- fix(budgets): meter cached prompt tokens ([#2579](https://github.com/everruns/everruns/pull/2579)) by [@chaliy](https://github.com/chaliy)
- fix(evals): bound oversized run case loading ([#2581](https://github.com/everruns/everruns/pull/2581)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): rate limit session forks ([#2580](https://github.com/everruns/everruns/pull/2580)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): buffer post-output guarded deltas ([#2578](https://github.com/everruns/everruns/pull/2578)) by [@chaliy](https://github.com/chaliy)
- fix(apps): harden schedule channel limits ([#2582](https://github.com/everruns/everruns/pull/2582)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): reject forged parent links ([#2577](https://github.com/everruns/everruns/pull/2577)) by [@chaliy](https://github.com/chaliy)
- feat(bashkit): egress-routed outbound HTTP behind enable_http config ([#2588](https://github.com/everruns/everruns/pull/2588)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump rust 1.96.0 to 1.96.1-slim-bookworm in /docker ([#2586](https://github.com/everruns/everruns/pull/2586)) by [@dependabot](https://github.com/apps/dependabot)
- chore(deps): bump rust 1.96.0-slim to 1.96.1-slim in /crates/worker ([#2585](https://github.com/everruns/everruns/pull/2585)) by [@dependabot](https://github.com/apps/dependabot)
- chore(deps): bump rust 1.96.0-slim to 1.96.1-slim in /crates/server ([#2584](https://github.com/everruns/everruns/pull/2584)) by [@dependabot](https://github.com/apps/dependabot)
- chore(deps): bump cargo group (ed25519-dalek 3, mlua 0.12, aws-smithy-types 1.6) ([#2589](https://github.com/everruns/everruns/pull/2589)) by [@dependabot](https://github.com/apps/dependabot)

## [0.17.4] - 2026-07-05

### Highlights

- **Streamlined Onboarding** — Durable onboarding-complete + resume state, sidebar-less setup flow ([#2560](https://github.com/everruns/everruns/pull/2560)).
- **Eval Run Comparison** — Compare eval runs side-by-side with regression highlighting ([#2531](https://github.com/everruns/everruns/pull/2531)).

### What's Changed

- test(llm): update Fireworks live model by [@chaliy](https://github.com/chaliy)
- chore(deps): bump fetchkit to 0.4.1 by [@chaliy](https://github.com/chaliy)
- feat(evals): add platform-capability Mira eval study ([#2564](https://github.com/everruns/everruns/pull/2564)) by [@chaliy](https://github.com/chaliy)
- test(ui): drop CSS-class change-detector tests ([#2563](https://github.com/everruns/everruns/pull/2563)) by [@chaliy](https://github.com/chaliy)
- test(server): drop trivial DTO serde round-trips in api ([#2562](https://github.com/everruns/everruns/pull/2562)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): cache-friendly Facts for dynamic context ([#2557](https://github.com/everruns/everruns/pull/2557)) by [@chaliy](https://github.com/chaliy)
- feat(onboarding): durable onboarding-complete + resume, sidebar-less setup ([#2560](https://github.com/everruns/everruns/pull/2560)) by [@chaliy](https://github.com/chaliy)
- chore(specs): index signup-experience-redesign-brief in README ([#2561](https://github.com/everruns/everruns/pull/2561)) by [@chaliy](https://github.com/chaliy)
- feat(ui): cross-run eval comparison with regression highlighting ([#2531](https://github.com/everruns/everruns/pull/2531)) by [@chaliy](https://github.com/chaliy)
- test(core): replace per-model profile mirrors with invariant ([#2558](https://github.com/everruns/everruns/pull/2558)) by [@chaliy](https://github.com/chaliy)
- test(core): prune capability metadata mirrors across modules ([#2551](https://github.com/everruns/everruns/pull/2551)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump the cargo group across 1 directory with 10 updates ([#2554](https://github.com/everruns/everruns/pull/2554)) by [@dependabot](https://github.com/apps/dependabot)
- test(llm-tests): skip live matrix on transient transport errors ([#2550](https://github.com/everruns/everruns/pull/2550)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump react and @types/react in /apps/ui ([#2538](https://github.com/everruns/everruns/pull/2538)) by [@dependabot](https://github.com/apps/dependabot)
- chore(deps): bump cmov from 0.5.3 to 0.5.4 in /examples/weekend-concierge-host in the cargo group across 1 directory ([#2553](https://github.com/everruns/everruns/pull/2553)) by [@dependabot](https://github.com/apps/dependabot)

## [0.17.3] - 2026-07-02

### What's Changed

- fix(ui): restore `pnpm.overrides`/`onlyBuiltDependencies` in `apps/ui/package.json`, accidentally dropped in v0.17.2, which broke the release Docker image build (`pnpm install --frozen-lockfile` lockfile-config mismatch) by [@chaliy](https://github.com/chaliy)

## [0.17.2] - 2026-07-02

### Highlights

- **Native Account Recovery** — Self-service account recovery flow with a branded onboarding shell ([#2541](https://github.com/everruns/everruns/pull/2541)).
- **Observers** — New Observers UI to create/edit observers with a scorer catalog and a per-observer Quality tab; `llm_judge` scorers referencing a model the org cannot use are now rejected ([#2524](https://github.com/everruns/everruns/pull/2524)).
- **Imported Eval Runs** — Ingest externally-executed eval runs and view them with attribution, transcript, and matrix ([#2516](https://github.com/everruns/everruns/pull/2516), [#2517](https://github.com/everruns/everruns/pull/2517)).
- **Chat Transcript Turn Navigation** — Step through turns with the keyboard and a new turn navigation rail ([#2542](https://github.com/everruns/everruns/pull/2542), [#2548](https://github.com/everruns/everruns/pull/2548)).
- **OpenRouter OAuth** — Connect OpenRouter directly from the Add Provider dialog ([#2519](https://github.com/everruns/everruns/pull/2519)).

### What's Changed

- feat(auth): native account recovery + branded onboarding shell ([#2541](https://github.com/everruns/everruns/pull/2541)) by [@chaliy](https://github.com/chaliy)
- feat(ui): keyboard turn stepping for chat transcript ([#2548](https://github.com/everruns/everruns/pull/2548)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add turn navigation rail to chat transcript ([#2542](https://github.com/everruns/everruns/pull/2542)) by [@chaliy](https://github.com/chaliy)
- feat(evals): async dataset export handle + e2e/tenant tests ([#2545](https://github.com/everruns/everruns/pull/2545)) by [@chaliy](https://github.com/chaliy)
- feat(evals): view imported eval runs — attribution, transcript, matrix ([#2517](https://github.com/everruns/everruns/pull/2517)) by [@chaliy](https://github.com/chaliy)
- feat(evals): ingest externally-executed eval runs ([#2516](https://github.com/everruns/everruns/pull/2516)) by [@chaliy](https://github.com/chaliy)
- feat(observers): Observers UI + llm_judge model-access validation ([#2524](https://github.com/everruns/everruns/pull/2524)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): align async tools with 2026 Tasks extension vocab ([#2544](https://github.com/everruns/everruns/pull/2544)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): auto-negotiate legacy, current, and 2026 RC protocols ([#2502](https://github.com/everruns/everruns/pull/2502)) by [@chaliy](https://github.com/chaliy)
- feat(ui): surface OpenRouter OAuth in Add Provider dialog ([#2519](https://github.com/everruns/everruns/pull/2519)) by [@chaliy](https://github.com/chaliy)
- feat(ui): reimplement create app page and detail identity editor ([#2521](https://github.com/everruns/everruns/pull/2521)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unify list empty states with shared EmptyState ([#2525](https://github.com/everruns/everruns/pull/2525)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add early-access banner with user-preferences store ([#2510](https://github.com/everruns/everruns/pull/2510)) by [@chaliy](https://github.com/chaliy)
- fix(security): collapse public-app & OAuth-signup enumeration leaks ([#2533](https://github.com/everruns/everruns/pull/2533)) by [@chaliy](https://github.com/chaliy)
- fix(security): route provider/LLM and MCP-OAuth HTTP through egress boundary ([#2530](https://github.com/everruns/everruns/pull/2530)) by [@chaliy](https://github.com/chaliy)
- fix(security): gate user hooks before validation ([#2496](https://github.com/everruns/everruns/pull/2496)) by [@chaliy](https://github.com/chaliy)
- fix(files): block network in HTML previews ([#2490](https://github.com/everruns/everruns/pull/2490)) by [@chaliy](https://github.com/chaliy)
- fix(local): secure local profile storage ([#2494](https://github.com/everruns/everruns/pull/2494)) by [@chaliy](https://github.com/chaliy)
- fix(api): register 13 plugin/marketplace handlers in OpenAPI ApiDoc ([#2523](https://github.com/everruns/everruns/pull/2523)) by [@chaliy](https://github.com/chaliy)
- fix(providers): allow OAuth provider creation without key ([#2500](https://github.com/everruns/everruns/pull/2500)) by [@chaliy](https://github.com/chaliy)
- fix(schedules): make schedule cap creation atomic ([#2492](https://github.com/everruns/everruns/pull/2492)) by [@chaliy](https://github.com/chaliy)
- fix(storage): delete replaced workspace file blobs ([#2491](https://github.com/everruns/everruns/pull/2491)) by [@chaliy](https://github.com/chaliy)
- fix(core): clear reasoning effort override per turn ([#2501](https://github.com/everruns/everruns/pull/2501)) by [@chaliy](https://github.com/chaliy)
- fix(rate-limit): gate grpc worker tool calls ([#2495](https://github.com/everruns/everruns/pull/2495)) by [@chaliy](https://github.com/chaliy)
- fix(server): idle sealed turns without input turn id ([#2497](https://github.com/everruns/everruns/pull/2497)) by [@chaliy](https://github.com/chaliy)
- fix(server): unify control-plane FS path normalization (EVE-670) ([#2505](https://github.com/everruns/everruns/pull/2505)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): fail open on malformed judge output ([#2493](https://github.com/everruns/everruns/pull/2493)) by [@chaliy](https://github.com/chaliy)
- fix(evals): avoid mutable scorer labels in exports ([#2499](https://github.com/everruns/everruns/pull/2499)) by [@chaliy](https://github.com/chaliy)
- fix(ui): lazy-load command palette data, gate evals on feature flag ([#2518](https://github.com/everruns/everruns/pull/2518)) by [@chaliy](https://github.com/chaliy)
- fix(ui): reword empty feature-flags state for SaaS users ([#2520](https://github.com/everruns/everruns/pull/2520)) by [@chaliy](https://github.com/chaliy)
- perf(worker): use native protobuf for session-task RPC payloads ([#2543](https://github.com/everruns/everruns/pull/2543)) by [@chaliy](https://github.com/chaliy)
- perf(durable): cut redundant event-log loads/replays and N+1 writes on hot path ([#2532](https://github.com/everruns/everruns/pull/2532)) by [@chaliy](https://github.com/chaliy)
- perf(ui): lazy-load trajectory/metrics deps, gate live-activity poll ([#2522](https://github.com/everruns/everruns/pull/2522)) by [@chaliy](https://github.com/chaliy)
- refactor: fold everruns-config into everruns-core ([#2547](https://github.com/everruns/everruns/pull/2547)) by [@chaliy](https://github.com/chaliy)
- refactor(drivers): golden streaming harness + shared accumulator ([#2546](https://github.com/everruns/everruns/pull/2546)) by [@chaliy](https://github.com/chaliy)
- refactor(drivers): extract shared provider-driver helpers (EVE-647) ([#2504](https://github.com/everruns/everruns/pull/2504)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): shared tool scaffolding, less boilerplate (EVE-646) ([#2507](https://github.com/everruns/everruns/pull/2507)) by [@chaliy](https://github.com/chaliy)
- refactor(core): adopt shared tool scaffolding in session_tasks/subagents ([#2528](https://github.com/everruns/everruns/pull/2528)) by [@chaliy](https://github.com/chaliy)
- refactor(core): collapse EventData identity into one tagged table ([#2487](https://github.com/everruns/everruns/pull/2487)) by [@chaliy](https://github.com/chaliy)
- refactor(observability): extract exporters from core (EVE-651) by [@chaliy](https://github.com/chaliy)
- refactor(errors): typed HTTP/fs/a2a error classification (EVE-645) by [@chaliy](https://github.com/chaliy)
- refactor(server): supervise startup background tasks by [@chaliy](https://github.com/chaliy)
- chore(deps): bump cargo group (19 updates) + migrate breaking APIs ([#2515](https://github.com/everruns/everruns/pull/2515)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump next from 16.2.6 to 16.2.9 in /apps/ui ([#2540](https://github.com/everruns/everruns/pull/2540)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump @base-ui/react from 1.5.0 to 1.6.0 in /apps/ui ([#2539](https://github.com/everruns/everruns/pull/2539)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump @rjsf/validator-ajv8 from 6.5.3 to 6.6.2 in /apps/ui ([#2537](https://github.com/everruns/everruns/pull/2537)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump sharp from 0.34.5 to 0.35.2 in /apps/docs ([#2536](https://github.com/everruns/everruns/pull/2536)) by [@dependabot](https://github.com/dependabot)

## [0.17.1] - 2026-06-27

### Highlights

- **Session Forking** — Sessions can now be forked into independent copies, enabling branched experimentation from any point in a conversation ([#2471](https://github.com/everruns/everruns/pull/2471)).
- **MCP Stateless Spec** — MCP endpoint now conforms to the 2026-07-28 stateless spec for improved interoperability ([#2484](https://github.com/everruns/everruns/pull/2484)).
- **Unified UI Layout** — Agents, Harnesses, Sessions, Dashboard, and Settings pages are now unified on a consistent five-zone layout ([#2476](https://github.com/everruns/everruns/pull/2476), [#2477](https://github.com/everruns/everruns/pull/2477), [#2478](https://github.com/everruns/everruns/pull/2478)).

### What's Changed

- test(ci): wire unrun server integration tests + enumeration guard ([#2482](https://github.com/everruns/everruns/pull/2482)) by [@chaliy](https://github.com/chaliy)
- fix(worker): finish TaskWorker migration by [@chaliy](https://github.com/chaliy)
- fix(storage): add repository conformance suite by [@chaliy](https://github.com/chaliy)
- feat(mcp): conform MCP endpoint to 2026-07-28 stateless spec ([#2484](https://github.com/everruns/everruns/pull/2484)) by [@chaliy](https://github.com/chaliy)
- docs(ui): adopt DESIGN.md for the Slate design system ([#2488](https://github.com/everruns/everruns/pull/2488)) by [@chaliy](https://github.com/chaliy)
- refactor(core): make MountFs the single workspace path resolver ([#2483](https://github.com/everruns/everruns/pull/2483)) by [@chaliy](https://github.com/chaliy)
- fix(examples): repair weekend-concierge-host compile against core API drift ([#2481](https://github.com/everruns/everruns/pull/2481)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): drop org from MCP OAuth consent page ([#2480](https://github.com/everruns/everruns/pull/2480)) by [@chaliy](https://github.com/chaliy)
- fix(auth): bind MCP OAuth consent to session, not a second cookie ([#2479](https://github.com/everruns/everruns/pull/2479)) by [@chaliy](https://github.com/chaliy)
- feat(core): mount-based workspace path resolver ([#2469](https://github.com/everruns/everruns/pull/2469)) by [@chaliy](https://github.com/chaliy)
- feat(ui): align Sessions, Dashboard, and Settings with the page language ([#2478](https://github.com/everruns/everruns/pull/2478)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unify building-block entity pages on the five-zone layout ([#2477](https://github.com/everruns/everruns/pull/2477)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unify Agents and Harnesses pages on a five-zone layout ([#2476](https://github.com/everruns/everruns/pull/2476)) by [@chaliy](https://github.com/chaliy)
- ci(security): gate Rust deps on RustSec advisories + fix git2 by [@chaliy](https://github.com/chaliy)
- refactor(internal-protocol): prefixed_id helper + role logs by [@chaliy](https://github.com/chaliy)
- fix(internal-protocol): surface conversion data loss instead of dropping silently ([#2470](https://github.com/everruns/everruns/pull/2470)) by [@chaliy](https://github.com/chaliy)
- perf(server): read-through cache for org feature-flag lookups ([#2468](https://github.com/everruns/everruns/pull/2468)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): fork a session into an independent copy ([#2471](https://github.com/everruns/everruns/pull/2471)) by [@chaliy](https://github.com/chaliy)
- feat(core): normalize token usage to disjoint cache buckets ([#2467](https://github.com/everruns/everruns/pull/2467)) by [@chaliy](https://github.com/chaliy)
- fix(llm): retry transient connection send errors across providers ([#2466](https://github.com/everruns/everruns/pull/2466)) by [@chaliy](https://github.com/chaliy)
- feat(guardrails): end-of-message output seam + moderation check ([#2465](https://github.com/everruns/everruns/pull/2465)) by [@chaliy](https://github.com/chaliy)
- feat(voice): per-connection realtime provider binding ([#2460](https://github.com/everruns/everruns/pull/2460)) by [@chaliy](https://github.com/chaliy)

## [0.17.0] - 2026-06-25

### Highlights

- **Sandboxed File Previews** — HTML and PDF files can now be previewed in a secure sandboxed viewer ([#2418](https://github.com/everruns/everruns/pull/2418)).
- **Security Hardening** — Broad security pass: constant-time auth comparisons, Argon2id parameter pinning, OAuth CSRF/replay protection, DNS-pinned webhook delivery (SSRF), and rate-limited OAuth client registration ([#2427](https://github.com/everruns/everruns/pull/2427), [#2432](https://github.com/everruns/everruns/pull/2432), [#2434](https://github.com/everruns/everruns/pull/2434), [#2435](https://github.com/everruns/everruns/pull/2435), [#2429](https://github.com/everruns/everruns/pull/2429)).
- **Per-domain Agent Permissions** — `OrgAgentsManage` is now split into fine-grained per-domain permissions for tighter access control ([#2451](https://github.com/everruns/everruns/pull/2451)).

### What's Changed

- fix(storage): harden bare-id repository methods (tenant isolation) ([#2458](https://github.com/everruns/everruns/pull/2458)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): settle idle turns from terminal events ([#2444](https://github.com/everruns/everruns/pull/2444)) by [@chaliy](https://github.com/chaliy)
- refactor(authz): split OrgAgentsManage into per-domain permissions ([#2451](https://github.com/everruns/everruns/pull/2451)) by [@chaliy](https://github.com/chaliy)
- fix(container-sandbox): secure Docker-host default; document runtime/egress assumptions ([#2450](https://github.com/everruns/everruns/pull/2450)) by [@chaliy](https://github.com/chaliy)
- perf(server,worker): mimalloc global allocator; bound history read ([#2457](https://github.com/everruns/everruns/pull/2457)) by [@chaliy](https://github.com/chaliy)
- perf(server): bound agent_version_metadata_cache with moka ([#2456](https://github.com/everruns/everruns/pull/2456)) by [@chaliy](https://github.com/chaliy)
- fix(storage): keep blob GC prefix scans delimited ([#2441](https://github.com/everruns/everruns/pull/2441)) by [@chaliy](https://github.com/chaliy)
- fix(usage): preserve aggregate effective cost ([#2446](https://github.com/everruns/everruns/pull/2446)) by [@chaliy](https://github.com/chaliy)
- fix(server): prune recorded task artifacts ([#2440](https://github.com/everruns/everruns/pull/2440)) by [@chaliy](https://github.com/chaliy)
- fix(server): enforce session caps in app channels ([#2439](https://github.com/everruns/everruns/pull/2439)) by [@chaliy](https://github.com/chaliy)
- fix(agents): persist parallel tool call updates ([#2443](https://github.com/everruns/everruns/pull/2443)) by [@chaliy](https://github.com/chaliy)
- fix(server): avoid canceling fired one-shot monitors ([#2445](https://github.com/everruns/everruns/pull/2445)) by [@chaliy](https://github.com/chaliy)
- fix(agents): bound custom check rules ([#2442](https://github.com/everruns/everruns/pull/2442)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add llms.txt generation and build-time link validation ([#2448](https://github.com/everruns/everruns/pull/2448)) by [@chaliy](https://github.com/chaliy)
- fix(ui): guard /dev showcase pages behind isDev ([#2449](https://github.com/everruns/everruns/pull/2449)) by [@chaliy](https://github.com/chaliy)
- fix(cli): validate OAuth CSRF state on login callback ([#2447](https://github.com/everruns/everruns/pull/2447)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): constant-time worker token compare; document TM-AUTHZ-002 ([#2436](https://github.com/everruns/everruns/pull/2436)) by [@chaliy](https://github.com/chaliy)
- fix(auth): rate-limit OAuth dynamic client registration ([#2435](https://github.com/everruns/everruns/pull/2435)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add suggested + recent quick links to search empty state ([#2438](https://github.com/everruns/everruns/pull/2438)) by [@chaliy](https://github.com/chaliy)
- fix(auth): use constant-time comparison for credential checks ([#2434](https://github.com/everruns/everruns/pull/2434)) by [@chaliy](https://github.com/chaliy)
- fix(auth): pin Argon2id password-hashing parameters ([#2432](https://github.com/everruns/everruns/pull/2432)) by [@chaliy](https://github.com/chaliy)
- fix(webhooks): pin DNS on task-webhook delivery (SSRF) ([#2429](https://github.com/everruns/everruns/pull/2429)) by [@chaliy](https://github.com/chaliy)
- fix(auth): fail closed on AUTH_MODE misconfiguration ([#2428](https://github.com/everruns/everruns/pull/2428)) by [@chaliy](https://github.com/chaliy)
- fix(auth): reject MCP OAuth authorization-code replay (TOCTOU) ([#2427](https://github.com/everruns/everruns/pull/2427)) by [@chaliy](https://github.com/chaliy)
- fix(email): avoid remote logo loads in templates ([#2424](https://github.com/everruns/everruns/pull/2424)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): reject client-side MCP tool names ([#2421](https://github.com/everruns/everruns/pull/2421)) by [@chaliy](https://github.com/chaliy)
- fix(core): avoid role-promoting tool errors ([#2420](https://github.com/everruns/everruns/pull/2420)) by [@chaliy](https://github.com/chaliy)
- fix(core): serialize session sandbox tools ([#2422](https://github.com/everruns/everruns/pull/2422)) by [@chaliy](https://github.com/chaliy)
- fix(ui): quote names in coding-agent prompts ([#2423](https://github.com/everruns/everruns/pull/2423)) by [@chaliy](https://github.com/chaliy)
- fix(storage): correct events message index predicate, prune indexes ([#2425](https://github.com/everruns/everruns/pull/2425)) by [@chaliy](https://github.com/chaliy)
- fix(session-file-system): make edit_file edits[]-only ([#2419](https://github.com/everruns/everruns/pull/2419)) by [@chaliy](https://github.com/chaliy)
- feat(files): add sandboxed HTML and PDF file previews ([#2418](https://github.com/everruns/everruns/pull/2418)) by [@chaliy](https://github.com/chaliy)

## [0.16.2] - 2026-06-23

### Highlights

- **First-class OAuth Credentials** — Providers now support typed multi-field credential schemas, enabling proper OAuth credential configuration ([#2413](https://github.com/everruns/everruns/pull/2413)).
- **MCP Always On** — The MCP endpoint is now always exposed; the `mcp_endpoint` feature flag has been removed ([#2395](https://github.com/everruns/everruns/pull/2395)).
- **Parallel Tool Calls** — New `parallel_tool_calls` preference unified across all drivers ([#2398](https://github.com/everruns/everruns/pull/2398)).

### What's Changed

- chore(deps): bump llmsim to 0.5.0 and gate it behind a feature ([#2416](https://github.com/everruns/everruns/pull/2416)) by [@chaliy](https://github.com/chaliy)
- feat(evals): join scorer names in dataset export ([#2415](https://github.com/everruns/everruns/pull/2415)) by [@chaliy](https://github.com/chaliy)
- feat(core): pluggable request auth for OpenResponses driver ([#2414](https://github.com/everruns/everruns/pull/2414)) by [@chaliy](https://github.com/chaliy)
- feat(providers): typed multi-field credential schemas (first-class OAuth) ([#2413](https://github.com/everruns/everruns/pull/2413)) by [@chaliy](https://github.com/chaliy)
- fix(release): correct provider core version-pins after default-features opt-out ([#2412](https://github.com/everruns/everruns/pull/2412)) by [@chaliy](https://github.com/chaliy)
- fix(loop-detection): interrupt repeated identical failed mutating calls ([#2411](https://github.com/everruns/everruns/pull/2411)) by [@chaliy](https://github.com/chaliy)
- fix(session-file-system): make edit_file mixed-mode error corrective ([#2410](https://github.com/everruns/everruns/pull/2410)) by [@chaliy](https://github.com/chaliy)
- fix(llm-tests): narrow quota detector for 429s ([#2408](https://github.com/everruns/everruns/pull/2408)) by [@chaliy](https://github.com/chaliy)
- fix(storage): use immutable session file blob keys ([#2407](https://github.com/everruns/everruns/pull/2407)) by [@chaliy](https://github.com/chaliy)
- fix(ard): enforce egress controls ([#2406](https://github.com/everruns/everruns/pull/2406)) by [@chaliy](https://github.com/chaliy)
- fix(orgs): require verified email for invites ([#2405](https://github.com/everruns/everruns/pull/2405)) by [@chaliy](https://github.com/chaliy)
- fix(providers): sign OAuth state cookie ([#2404](https://github.com/everruns/everruns/pull/2404)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): reject client-side MCP allowlist entries ([#2403](https://github.com/everruns/everruns/pull/2403)) by [@chaliy](https://github.com/chaliy)
- fix(ui): preserve AG-UI endpoint auth on edit ([#2402](https://github.com/everruns/everruns/pull/2402)) by [@chaliy](https://github.com/chaliy)
- refactor(providers): shed core default features in thin provider crates ([#2401](https://github.com/everruns/everruns/pull/2401)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): always expose MCP endpoint, drop mcp_endpoint flag ([#2395](https://github.com/everruns/everruns/pull/2395)) by [@chaliy](https://github.com/chaliy)
- refactor(core): gate telemetry, a2a, web-fetch behind cargo features ([#2399](https://github.com/everruns/everruns/pull/2399)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): parallel_tool_calls preference, unify drivers ([#2398](https://github.com/everruns/everruns/pull/2398)) by [@chaliy](https://github.com/chaliy)
- refactor(api): expose feature flags as untyped key→bool map ([#2396](https://github.com/everruns/everruns/pull/2396)) by [@chaliy](https://github.com/chaliy)
- feat(evals): reward-labeled trajectory dataset export from eval runs ([#2397](https://github.com/everruns/everruns/pull/2397)) by [@chaliy](https://github.com/chaliy)
- fix(chat): enforce global_chat flag end-to-end; clarify description ([#2392](https://github.com/everruns/everruns/pull/2392)) by [@chaliy](https://github.com/chaliy)
- feat(schedules): min cron interval + per-org cap on session schedules ([#2394](https://github.com/everruns/everruns/pull/2394)) by [@chaliy](https://github.com/chaliy)
- refactor(apps): graduate apps.detailV2, remove legacy app detail ([#2393](https://github.com/everruns/everruns/pull/2393)) by [@chaliy](https://github.com/chaliy)
- docs(feature-flags): humanize org settings flag descriptions ([#2390](https://github.com/everruns/everruns/pull/2390)) by [@chaliy](https://github.com/chaliy)
- feat(ui): integration guides for agents, harnesses, and apps ([#2389](https://github.com/everruns/everruns/pull/2389)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): unify list cards behind shared EntityCard ([#2391](https://github.com/everruns/everruns/pull/2391)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add shared SearchInput for consistent search box height ([#2388](https://github.com/everruns/everruns/pull/2388)) by [@chaliy](https://github.com/chaliy)
- fix(ui): use accent variant for MCP Servers add button ([#2387](https://github.com/everruns/everruns/pull/2387)) by [@chaliy](https://github.com/chaliy)
- test(runtime): make crate-level example a tested doctest ([#2386](https://github.com/everruns/everruns/pull/2386)) by [@chaliy](https://github.com/chaliy)
- fix(providers): classify provider type validation ([#2409](https://github.com/everruns/everruns/pull/2409)) by [@chaliy](https://github.com/chaliy)

## [0.16.1] - 2026-06-21

### What's Changed

- feat(server): add per-org limits for harnesses, agents, sessions ([#2383](https://github.com/everruns/everruns/pull/2383)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add Create app shortcut from harness and agent pages ([#2382](https://github.com/everruns/everruns/pull/2382)) by [@chaliy](https://github.com/chaliy)
- feat(providers): link session chats to provider trace/logs ([#2380](https://github.com/everruns/everruns/pull/2380)) by [@chaliy](https://github.com/chaliy)
- feat(ui): per-task drill-down on the session Tasks tab ([#2378](https://github.com/everruns/everruns/pull/2378)) by [@chaliy](https://github.com/chaliy)
- feat(onboarding): add OSS-owned zero-org onboarding surface ([#2379](https://github.com/everruns/everruns/pull/2379)) by [@chaliy](https://github.com/chaliy)
- feat(orgs): add org creation policy extension point for wrappers ([#2375](https://github.com/everruns/everruns/pull/2375)) by [@chaliy](https://github.com/chaliy)
- fix(worker): bound worker concurrency, claim batch, and idle polling ([#2373](https://github.com/everruns/everruns/pull/2373)) by [@chaliy](https://github.com/chaliy)
- fix(durable): serve system-health totals from maintained counters ([#2372](https://github.com/everruns/everruns/pull/2372)) by [@chaliy](https://github.com/chaliy)
- fix(brand): center all logos on the rings centroid ([#2370](https://github.com/everruns/everruns/pull/2370)) by [@chaliy](https://github.com/chaliy)
- chore(events): retire subagent.* event types ([#2376](https://github.com/everruns/everruns/pull/2376)) by [@chaliy](https://github.com/chaliy)
- docs(skills): add public everruns-runtime skill ([#2381](https://github.com/everruns/everruns/pull/2381)) by [@chaliy](https://github.com/chaliy)
- docs: expand OpenRouter docs and add server-tools capability ([#2374](https://github.com/everruns/everruns/pull/2374)) by [@chaliy](https://github.com/chaliy)
- ci: cut redundant rust recompiles to speed up cold CI ([#2371](https://github.com/everruns/everruns/pull/2371)) by [@chaliy](https://github.com/chaliy)

## [0.16.0] - 2026-06-20

### Highlights

- **Tool-Call Repair** — New capability that automatically detects and repairs malformed tool calls from LLMs, improving reliability across providers ([#2364](https://github.com/everruns/everruns/pull/2364)).
- **OAuth Connect Flow for Providers** — Connect model providers via OAuth, starting with OpenRouter ([#2353](https://github.com/everruns/everruns/pull/2353)).
- **Organization Invitations** — OSS-owned org invitation system with optional email delivery ([#2350](https://github.com/everruns/everruns/pull/2350)).
- **Fireworks AI Provider** — New built-in Fireworks AI provider for open models (Llama, Qwen, DeepSeek, and more) with automatic model discovery ([#2344](https://github.com/everruns/everruns/pull/2344)).
- **Environment Credential Injection** — Inject provider credentials via `CredentialProvider` for zero-config deployments ([#2348](https://github.com/everruns/everruns/pull/2348)).

### What's Changed

- feat(capabilities): tool-call repair capability for malformed tool calls ([#2364](https://github.com/everruns/everruns/pull/2364)) by [@chaliy](https://github.com/chaliy)
- feat(providers): OAuth connect flow for drivers, starting with OpenRouter ([#2353](https://github.com/everruns/everruns/pull/2353)) by [@chaliy](https://github.com/chaliy)
- feat(email): app-styled minimal template and branded basic template ([#2354](https://github.com/everruns/everruns/pull/2354)) by [@chaliy](https://github.com/chaliy)
- feat(orgs): add OSS-owned organization invitations with optional email ([#2350](https://github.com/everruns/everruns/pull/2350)) by [@chaliy](https://github.com/chaliy)
- feat(local): publish everruns-local crate to crates.io ([#2351](https://github.com/everruns/everruns/pull/2351)) by [@chaliy](https://github.com/chaliy)
- feat(providers): inject env credentials via CredentialProvider ([#2348](https://github.com/everruns/everruns/pull/2348)) by [@chaliy](https://github.com/chaliy)
- feat(fireworks): add Fireworks AI provider with model sync ([#2344](https://github.com/everruns/everruns/pull/2344)) by [@chaliy](https://github.com/chaliy)
- fix(email): point basic template logo at hosted 64px PNG ([#2367](https://github.com/everruns/everruns/pull/2367)) by [@chaliy](https://github.com/chaliy)
- fix(context): make infinity anchor opt-in ([#2360](https://github.com/everruns/everruns/pull/2360)) by [@chaliy](https://github.com/chaliy)
- fix(events): allow transcript repair filters ([#2362](https://github.com/everruns/everruns/pull/2362)) by [@chaliy](https://github.com/chaliy)
- fix(core): sanitize narrated fetch URLs ([#2358](https://github.com/everruns/everruns/pull/2358)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): scope mcp guardrail invoker ([#2357](https://github.com/everruns/everruns/pull/2357)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve capability narration hooks in act execution ([#2363](https://github.com/everruns/everruns/pull/2363)) by [@chaliy](https://github.com/chaliy)
- fix(infinity-context): cap anchored head messages ([#2359](https://github.com/everruns/everruns/pull/2359)) by [@chaliy](https://github.com/chaliy)
- fix(core): preserve legacy agent run records ([#2361](https://github.com/everruns/everruns/pull/2361)) by [@chaliy](https://github.com/chaliy)
- fix(openrouter): admin-gate server web tools ([#2356](https://github.com/everruns/everruns/pull/2356)) by [@chaliy](https://github.com/chaliy)
- chore(security): adopt security policy, testing spec, and process wiring ([#2368](https://github.com/everruns/everruns/pull/2368)) by [@chaliy](https://github.com/chaliy)
- docs,ui: use official OpenRouter logo ([#2355](https://github.com/everruns/everruns/pull/2355)) by [@chaliy](https://github.com/chaliy)
- docs,ui: fix OpenRouter icon and add provider sidebar icons ([#2352](https://github.com/everruns/everruns/pull/2352)) by [@chaliy](https://github.com/chaliy)
- docs(fireworks): provider guide, publishable README, UI taglines ([#2349](https://github.com/everruns/everruns/pull/2349)) by [@chaliy](https://github.com/chaliy)
- docs: add model providers section under integrations ([#2343](https://github.com/everruns/everruns/pull/2343)) by [@chaliy](https://github.com/chaliy)
- docs: point README links to docs.everruns.com and fill doc gaps ([#2341](https://github.com/everruns/everruns/pull/2341)) by [@chaliy](https://github.com/chaliy)

## [0.15.0] - 2026-06-19

### Highlights

- **Knowledge Indexes** - Source-backed semantic search with OKF-backed knowledge bases, enabling structured retrieval across indexed sources.
- **Agentic Resource Discovery (ARD)** - New ARD client capability for discovering and connecting to agentic resources ([#2324](https://github.com/everruns/everruns/pull/2324)).
- **everruns-local Crate** - Embedded host backends for local development and testing ([#2298](https://github.com/everruns/everruns/pull/2298)).
- **MCP External Guardrails** - New `mcp` check type for external guardrail validation over scoped MCP connections ([#2302](https://github.com/everruns/everruns/pull/2302)).
- **LLM Judge Guardrails** - New `llm_judge` check type for tool stages, enabling LLM-evaluated guardrail policies ([#2229](https://github.com/everruns/everruns/pull/2229)).
- **Session Task Retention** - Retention TTL and reaper cleanup for terminal tasks ([#2318](https://github.com/everruns/everruns/pull/2318)).
- **Org-Scoped Task Listing** - New endpoint for listing session tasks scoped to an organization (EVE-583) ([#2339](https://github.com/everruns/everruns/pull/2339)).

### What's Changed

- feat(session-tasks): org-scoped task listing endpoint (EVE-583) ([#2339](https://github.com/everruns/everruns/pull/2339)) by [@chaliy](https://github.com/chaliy)
- fix(mai): validate OAuth authority URLs ([#2337](https://github.com/everruns/everruns/pull/2337)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): support provider-executed server tools ([#2330](https://github.com/everruns/everruns/pull/2330)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump everruns-sdk to 0.1.10 ([#2329](https://github.com/everruns/everruns/pull/2329)) by [@chaliy](https://github.com/chaliy)
- feat(durable): seal non-progressing turns (EVE-534) ([#2336](https://github.com/everruns/everruns/pull/2336)) by [@chaliy](https://github.com/chaliy)
- feat(runtime): plumb request-level parallel_tool_calls (EVE-598) ([#2335](https://github.com/everruns/everruns/pull/2335)) by [@chaliy](https://github.com/chaliy)
- fix(auth): scope MCP OAuth tokens to /mcp resource (EVE-596) ([#2333](https://github.com/everruns/everruns/pull/2333)) by [@chaliy](https://github.com/chaliy)
- fix(ci): skip live LLM tests when provider quota is exhausted ([#2334](https://github.com/everruns/everruns/pull/2334)) by [@chaliy](https://github.com/chaliy)
- fix(budgets): cache-aware cost estimation (EVE-599) by [@chaliy](https://github.com/chaliy)
- fix(agents): throttle LLM analysis ([#2310](https://github.com/everruns/everruns/pull/2310)) by [@chaliy](https://github.com/chaliy)
- fix(core): avoid completing idle subagents ([#2316](https://github.com/everruns/everruns/pull/2316)) by [@chaliy](https://github.com/chaliy)
- fix(guardrails): prioritize tool output checks ([#2309](https://github.com/everruns/everruns/pull/2309)) by [@chaliy](https://github.com/chaliy)
- fix(openresponses): reject dangling function calls after compaction ([#2328](https://github.com/everruns/everruns/pull/2328)) by [@chaliy](https://github.com/chaliy)
- fix(openrouter): route workspace checks through egress ([#2307](https://github.com/everruns/everruns/pull/2307)) by [@chaliy](https://github.com/chaliy)
- fix(model-scout): enforce probe guardrails ([#2308](https://github.com/everruns/everruns/pull/2308)) by [@chaliy](https://github.com/chaliy)
- fix(openrouter): delay failed generation reconciliation ([#2312](https://github.com/everruns/everruns/pull/2312)) by [@chaliy](https://github.com/chaliy)
- fix(llm): drop orphan tool results for stateless providers ([#2317](https://github.com/everruns/everruns/pull/2317)) by [@chaliy](https://github.com/chaliy)
- fix(providers): reject empty provider types ([#2315](https://github.com/everruns/everruns/pull/2315)) by [@chaliy](https://github.com/chaliy)
- feat(knowledge-bases): adopt Open Knowledge Format (OKF) ([#2321](https://github.com/everruns/everruns/pull/2321)) by [@chaliy](https://github.com/chaliy)
- feat(ard): Agentic Resource Discovery (ARD) client capability ([#2324](https://github.com/everruns/everruns/pull/2324)) by [@chaliy](https://github.com/chaliy)
- chore: use SeaweedFS instead of MinIO for S3 blob backend ([#2323](https://github.com/everruns/everruns/pull/2323)) by [@chaliy](https://github.com/chaliy)
- fix(deps): patch Dependabot npm vulnerabilities (undici, dompurify) ([#2326](https://github.com/everruns/everruns/pull/2326)) by [@chaliy](https://github.com/chaliy)
- fix(knowledge-indexes): rebind source update owner ([#2304](https://github.com/everruns/everruns/pull/2304)) by [@chaliy](https://github.com/chaliy)
- fix(observers): bound scorer configuration ([#2311](https://github.com/everruns/everruns/pull/2311)) by [@chaliy](https://github.com/chaliy)
- fix(fs): escape display root in prompt ([#2306](https://github.com/everruns/everruns/pull/2306)) by [@chaliy](https://github.com/chaliy)
- fix(worker): provide file store for task reattach ([#2314](https://github.com/everruns/everruns/pull/2314)) by [@chaliy](https://github.com/chaliy)
- fix(openresponses): surface reasoning summaries as text by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): retention TTL + reaper cleanup for terminal tasks ([#2318](https://github.com/everruns/everruns/pull/2318)) by [@chaliy](https://github.com/chaliy)
- fix(hooks): preserve commit signing by default ([#2313](https://github.com/everruns/everruns/pull/2313)) by [@chaliy](https://github.com/chaliy)
- fix(deps): patch Dependabot-flagged npm vulnerabilities ([#2303](https://github.com/everruns/everruns/pull/2303)) by [@chaliy](https://github.com/chaliy)
- feat(guardrails): mcp check type — external guardrail over scoped MCP ([#2302](https://github.com/everruns/everruns/pull/2302)) by [@chaliy](https://github.com/chaliy)
- feat(guardrails): llm_judge check type for tool stages ([#2229](https://github.com/everruns/everruns/pull/2229)) by [@chaliy](https://github.com/chaliy)
- feat(storage): garbage-collect orphaned object-storage blobs ([#2300](https://github.com/everruns/everruns/pull/2300)) by [@chaliy](https://github.com/chaliy)
- feat(core): mid-turn reasoning-effort changes within a single run_turn ([#2299](https://github.com/everruns/everruns/pull/2299)) by [@chaliy](https://github.com/chaliy)
- feat(local): everruns-local crate for embedded host backends ([#2298](https://github.com/everruns/everruns/pull/2298)) by [@chaliy](https://github.com/chaliy)
- feat(knowledge): Knowledge Indexes — source-backed semantic search by [@chaliy](https://github.com/chaliy)
- feat(harnesses): make base system prompt optional ([#2296](https://github.com/everruns/everruns/pull/2296)) by [@chaliy](https://github.com/chaliy)
- fix(core): detect repeated read range loops by [@chaliy](https://github.com/chaliy)
- fix(core): keep paginated reads in model view by [@chaliy](https://github.com/chaliy)
- fix(docs): render GFM tables in .mdx pages ([#2293](https://github.com/everruns/everruns/pull/2293)) by [@chaliy](https://github.com/chaliy)
- docs(runtime): add runnable OpenAI example and clarify provider setup ([#2291](https://github.com/everruns/everruns/pull/2291)) by [@chaliy](https://github.com/chaliy)

## [0.14.0] - 2026-06-17

### Highlights

- **Microsoft MAI Provider** - New Azure AI Foundry integration with OAuth authentication, bringing Microsoft's MAI model family to the provider ecosystem ([#2269](https://github.com/everruns/everruns/pull/2269)).
- **S3-Compatible Object Storage** - Optional S3-compatible blob backend for file attachments, enabling flexible deployment with external storage ([#2286](https://github.com/everruns/everruns/pull/2286)).
- **Org-Level Default Provider Resolution** - Configure per-service default providers at the organization level for more precise routing control ([#2288](https://github.com/everruns/everruns/pull/2288)).
- **LLM-as-Judge Online Scorer** - Automatic quality scoring of production sessions using LLM evaluation for continuous observability ([#2263](https://github.com/everruns/everruns/pull/2263)).

### What's Changed

- refactor(narration): tool-owned narration with capability default ([#2289](https://github.com/everruns/everruns/pull/2289)) by [@chaliy](https://github.com/chaliy)
- feat(providers): org-level default-provider-per-service resolution tier ([#2288](https://github.com/everruns/everruns/pull/2288)) by [@chaliy](https://github.com/chaliy)
- feat(storage): optional S3-compatible object-storage blob backend ([#2286](https://github.com/everruns/everruns/pull/2286)) by [@chaliy](https://github.com/chaliy)
- feat(agents): populate token usage in health-check summary ([#2287](https://github.com/everruns/everruns/pull/2287)) by [@chaliy](https://github.com/chaliy)
- feat(agents): show latest health-check run on agent editor mount ([#2285](https://github.com/everruns/everruns/pull/2285)) by [@chaliy](https://github.com/chaliy)
- fix(agents): reap interrupted agent health-check runs ([#2281](https://github.com/everruns/everruns/pull/2281)) by [@chaliy](https://github.com/chaliy)
- feat(infinity-context): load head+tail so task anchor survives long histories ([#2278](https://github.com/everruns/everruns/pull/2278)) by [@chaliy](https://github.com/chaliy)
- fix(drivers): preserve agent system prompt with multiple system messages ([#2279](https://github.com/everruns/everruns/pull/2279)) by [@chaliy](https://github.com/chaliy)
- feat(mai): add Microsoft MAI provider (Azure AI Foundry) with OAuth ([#2269](https://github.com/everruns/everruns/pull/2269)) by [@chaliy](https://github.com/chaliy)
- feat(observers): LLM-as-judge scorer for online scoring ([#2263](https://github.com/everruns/everruns/pull/2263)) by [@chaliy](https://github.com/chaliy)
- feat(auth): improve MCP OAuth consent page by [@chaliy](https://github.com/chaliy)
- feat(openrouter): add attribution headers by [@chaliy](https://github.com/chaliy)
- fix(openrouter): honor provider rate limit reset by [@chaliy](https://github.com/chaliy)
- fix(web-fetch): block cross-host redirects under system allowlist ([#2249](https://github.com/everruns/everruns/pull/2249)) by [@chaliy](https://github.com/chaliy)
- fix(core): keep DNS pinning inside egress policy ([#2239](https://github.com/everruns/everruns/pull/2239)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): reserve active turn slots ([#2251](https://github.com/everruns/everruns/pull/2251)) by [@chaliy](https://github.com/chaliy)
- fix(core): prevent orphaned A2A background runs ([#2241](https://github.com/everruns/everruns/pull/2241)) by [@chaliy](https://github.com/chaliy)
- fix(evals): make run cap enforcement atomic ([#2252](https://github.com/everruns/everruns/pull/2252)) by [@chaliy](https://github.com/chaliy)
- fix(feature-flags): enforce org-scoped runtime gates ([#2250](https://github.com/everruns/everruns/pull/2250)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): bound stale tool cache serving ([#2248](https://github.com/everruns/everruns/pull/2248)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): bound MCP discovery cache ([#2247](https://github.com/everruns/everruns/pull/2247)) by [@chaliy](https://github.com/chaliy)
- fix(core): bound tool output distillation work ([#2245](https://github.com/everruns/everruns/pull/2245)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): cap full tarball extraction size ([#2244](https://github.com/everruns/everruns/pull/2244)) by [@chaliy](https://github.com/chaliy)
- fix(core): prevent raw error details in fallback text ([#2243](https://github.com/everruns/everruns/pull/2243)) by [@chaliy](https://github.com/chaliy)
- fix(agents): cap preview check findings ([#2242](https://github.com/everruns/everruns/pull/2242)) by [@chaliy](https://github.com/chaliy)
- fix(security): reserve internal session kv keys ([#2237](https://github.com/everruns/everruns/pull/2237)) by [@chaliy](https://github.com/chaliy)
- fix(worker): preserve A2A reattach network ACL ([#2236](https://github.com/everruns/everruns/pull/2236)) by [@chaliy](https://github.com/chaliy)
- fix(server): fail closed on missing task harness ACL ([#2235](https://github.com/everruns/everruns/pull/2235)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): enforce workspace write boundary on legacy alias ([#2234](https://github.com/everruns/everruns/pull/2234)) by [@chaliy](https://github.com/chaliy)
- fix(agents): gate health checks on session limits ([#2232](https://github.com/everruns/everruns/pull/2232)) by [@chaliy](https://github.com/chaliy)
- fix(core): keep base system prompt before capabilities by [@chaliy](https://github.com/chaliy)

## [0.13.0] - 2026-06-15

### Highlights

- **Outbound Webhooks for Task Transitions** - Receive webhook callbacks when session tasks reach terminal states (EVE-579).
- **Agent Behavioral Health Checks** - Phase 3 of automatic behavioral health monitoring, adding scheduled health session runs and scoring for agents ([#2205](https://github.com/everruns/everruns/pull/2205)).

### What's Changed

- feat(agents): behavioral health checks (phase 3) ([#2205](https://github.com/everruns/everruns/pull/2205)) by [@chaliy](https://github.com/chaliy)
- fix(openrouter): keep reasoning private by default by [@chaliy](https://github.com/chaliy)
- feat(harnesses): add embedder_metadata for LLM and observability flows by [@chaliy](https://github.com/chaliy)
- feat(openrouter): reconcile authoritative generation cost and usage by [@chaliy](https://github.com/chaliy)
- fix(openrouter): handle Responses [DONE] sentinel + live coverage ([#2225](https://github.com/everruns/everruns/pull/2225)) by [@chaliy](https://github.com/chaliy)
- fix(context): anchor original task in infinity_context + compaction ([#2224](https://github.com/everruns/everruns/pull/2224)) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): outbound webhooks on terminal task transitions (EVE-579) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): re-attach orphaned background_tool runs after worker loss ([#2222](https://github.com/everruns/everruns/pull/2222)) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): cursor pagination for task messages (EVE-582) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): monitors execute probe tools directly on schedule fire ([#2221](https://github.com/everruns/everruns/pull/2221)) by [@chaliy](https://github.com/chaliy)
- feat(session-tasks): reject POST /messages for subagent-kind tasks ([#2219](https://github.com/everruns/everruns/pull/2219)) by [@chaliy](https://github.com/chaliy)
- refactor(openrouter): move request augmentation behind core seam ([#2218](https://github.com/everruns/everruns/pull/2218)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): session tracking + extract dedicated crate ([#2217](https://github.com/everruns/everruns/pull/2217)) by [@chaliy](https://github.com/chaliy)
- fix(fs): expose real disk workspace display paths ([#2216](https://github.com/everruns/everruns/pull/2216)) by [@chaliy](https://github.com/chaliy)

## [0.12.0] - 2026-06-13

### Highlights

- **Adoptable Guardrail Gallery** - Browse and adopt guardrail configurations from a curated gallery ([#2214](https://github.com/everruns/everruns/pull/2214)).
- **Plugins Subsystem** - Marketplace of plugins with install management for orgs ([#2150](https://github.com/everruns/everruns/pull/2150)).
- **OpenRouter Advanced Routing** - Capacity strategies (shared/BYOK-first/BYOK-only), quality routing presets, workspace inspection, and model scout benchmark (EVE-564, EVE-565, EVE-566, [#2204](https://github.com/everruns/everruns/pull/2204), [#2206](https://github.com/everruns/everruns/pull/2206)).
- **Workspace-Attached Sessions** - Sessions now attach to a shared workspace with unified file I/O and move/copy/download ([#2207](https://github.com/everruns/everruns/pull/2207)).
- **Config-Driven Guardrails** - Deterministic guardrail capability configurable per workspace ([#2193](https://github.com/everruns/everruns/pull/2193)).
- **Hosted Tool Search** - `claude_tool_search` available as a hosted capability with name-weighted ranking ([#2166](https://github.com/everruns/everruns/pull/2166)).
- **Online Session Scoring** - Automatic online scoring of production sessions for quality observability ([#2191](https://github.com/everruns/everruns/pull/2191)).

### What's Changed

- feat(guardrails): adoptable guardrail gallery (EVE-571) ([#2214](https://github.com/everruns/everruns/pull/2214)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): add provider-quality routing presets (EVE-564) by [@chaliy](https://github.com/chaliy)
- refactor(server): drop dead subagent_name/task columns from spawn handles ([#2209](https://github.com/everruns/everruns/pull/2209)) by [@chaliy](https://github.com/chaliy)
- feat(providers): service-kind resolution + voice realtime rerouting (phase 4) ([#2210](https://github.com/everruns/everruns/pull/2210)) by [@chaliy](https://github.com/chaliy)
- fix(ui): address session file browser by the session's workspace_id ([#2212](https://github.com/everruns/everruns/pull/2212)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): add capacity strategy (shared/BYOK-first/BYOK-only) (EVE-565) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): add workspace inspection and policy compatibility tools (EVE-566) by [@chaliy](https://github.com/chaliy)
- feat(workspace): attach sessions to a shared workspace + re-key I/O ([#2207](https://github.com/everruns/everruns/pull/2207)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): expose optional plugin-style provider capabilities ([#2206](https://github.com/everruns/everruns/pull/2206)) by [@chaliy](https://github.com/chaliy)
- feat(openrouter): add model scout benchmark blueprint ([#2204](https://github.com/everruns/everruns/pull/2204)) by [@chaliy](https://github.com/chaliy)
- refactor(api): rename provider/model routes to /v1/providers,/v1/models ([#2188](https://github.com/everruns/everruns/pull/2188)) by [@chaliy](https://github.com/chaliy)
- refactor: retire sessions.subagent_* columns for the task registry ([#2202](https://github.com/everruns/everruns/pull/2202)) by [@chaliy](https://github.com/chaliy)
- feat(guardrails): config-driven deterministic guardrail capability ([#2193](https://github.com/everruns/everruns/pull/2193)) by [@chaliy](https://github.com/chaliy)
- feat(agents): on-demand LLM analysis of agent configs (phase 2) ([#2194](https://github.com/everruns/everruns/pull/2194)) by [@chaliy](https://github.com/chaliy)
- refactor(providers): rename LLM types, modules, and DB tables (phase 2) by [@chaliy](https://github.com/chaliy)
- feat(providers): add EmbeddingsDriver trait and knowledge-base embedding config (phase 6) by [@chaliy](https://github.com/chaliy)
- fix(auth): require proxy secret for mTLS endpoint auth (EVE-545) by [@chaliy](https://github.com/chaliy)
- feat(observers): online scoring of production sessions (Phase 1) ([#2191](https://github.com/everruns/everruns/pull/2191)) by [@chaliy](https://github.com/chaliy)
- refactor(providers): rename ConnectionProvider → Connector across core, server, and plugin crates (phase 5) by [@chaliy](https://github.com/chaliy)
- refactor(core): retire legacy per-kind task tools for generic ones ([#2196](https://github.com/everruns/everruns/pull/2196)) by [@chaliy](https://github.com/chaliy)
- test(policy): enforce SESSION_VIEW on ListSessionFiles and GetSessionFile (EVE-551) by [@chaliy](https://github.com/chaliy)
- feat(workspace): fs move/copy/download, workspace_id, file rename ([#2189](https://github.com/everruns/everruns/pull/2189)) by [@chaliy](https://github.com/chaliy)
- refactor(core): nest web_fetch egress transport as a submodule ([#2195](https://github.com/everruns/everruns/pull/2195)) by [@chaliy](https://github.com/chaliy)
- fix(auth): validate expires_in_days range for personal access tokens by [@chaliy](https://github.com/chaliy)
- feat(core): configurable multi-scope skills capability ([#2185](https://github.com/everruns/everruns/pull/2185)) by [@chaliy](https://github.com/chaliy)
- fix(files): encode special chars in workspace filesystem URL paths (EVE-558, EVE-555) by [@chaliy](https://github.com/chaliy)
- fix(chat): validate MCP card URI, size, and tool provenance before rendering ([#2187](https://github.com/everruns/everruns/pull/2187)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): regroup capabilities into Core and Sandboxes ([#2183](https://github.com/everruns/everruns/pull/2183)) by [@chaliy](https://github.com/chaliy)
- fix(apps): gate publish/unpublish/archive on app.dangerous ([#2186](https://github.com/everruns/everruns/pull/2186)) by [@chaliy](https://github.com/chaliy)
- feat(mcp-servers): add custom headers management UI ([#2184](https://github.com/everruns/everruns/pull/2184)) by [@chaliy](https://github.com/chaliy)
- fix(ui): guard against invalid scheduledAt in formatScheduledDateTime (EVE-559) by [@chaliy](https://github.com/chaliy)
- fix(mcp-servers): redact header values in API responses (EVE-550) by [@chaliy](https://github.com/chaliy)
- fix(schedule): fix default cron and add minimum-interval validation by [@chaliy](https://github.com/chaliy)
- feat(cli): render session task lifecycle events in session follow ([#2177](https://github.com/everruns/everruns/pull/2177)) by [@chaliy](https://github.com/chaliy)
- fix(authz): add EVAL_VIEW and SESSION_VIEW to GET read commands by [@chaliy](https://github.com/chaliy)
- fix(security): allowlist raster MIME types in read-file image rendering by [@chaliy](https://github.com/chaliy)
- fix(core): match profile-key vendor segment case-insensitively ([#2174](https://github.com/everruns/everruns/pull/2174)) by [@chaliy](https://github.com/chaliy)
- fix(evals): CreateEvalRun requires session permission (EVE-549) by [@chaliy](https://github.com/chaliy)
- feat(server): seed default everruns plugin marketplace per org ([#2160](https://github.com/everruns/everruns/pull/2160)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): align all built-in capabilities to pub const ID pattern ([#2173](https://github.com/everruns/everruns/pull/2173)) by [@chaliy](https://github.com/chaliy)
- feat(core): driver descriptors with services and credential schemas ([#2170](https://github.com/everruns/everruns/pull/2170)) by [@chaliy](https://github.com/chaliy)
- feat(tool-search): add hosted claude_tool_search and wire into auto ([#2166](https://github.com/everruns/everruns/pull/2166)) by [@chaliy](https://github.com/chaliy)
- refactor(core): rename LlmDriver to ChatDriver ([#2169](https://github.com/everruns/everruns/pull/2169)) by [@chaliy](https://github.com/chaliy)
- feat(agents): advisory agent config checks (phase 1) ([#2167](https://github.com/everruns/everruns/pull/2167)) by [@chaliy](https://github.com/chaliy)
- feat(server): reconcile monitors when schedules are canceled ([#2164](https://github.com/everruns/everruns/pull/2164)) by [@chaliy](https://github.com/chaliy)
- fix(tool-search): preserve deferred schemas across serde ([#2135](https://github.com/everruns/everruns/pull/2135)) by [@chaliy](https://github.com/chaliy)
- fix(core): ignore empty stream events for stall timeout ([#2132](https://github.com/everruns/everruns/pull/2132)) by [@chaliy](https://github.com/chaliy)
- feat(core): compact model-facing output after tool output persistence ([#2151](https://github.com/everruns/everruns/pull/2151)) by [@chaliy](https://github.com/chaliy)
- feat(core): open LLM driver model for embedder-defined providers ([#2161](https://github.com/everruns/everruns/pull/2161)) by [@chaliy](https://github.com/chaliy)
- feat(workspace): decouple Workspace from Session, add fs API by [@chaliy](https://github.com/chaliy)
- feat(server): invoke task executors from the session-task API ([#2159](https://github.com/everruns/everruns/pull/2159)) by [@chaliy](https://github.com/chaliy)
- feat(core): semantic driver errors and error-disclosure capability ([#2147](https://github.com/everruns/everruns/pull/2147)) by [@chaliy](https://github.com/chaliy)
- feat(worker): re-attach orphaned external_agent tasks ([#2153](https://github.com/everruns/everruns/pull/2153)) by [@chaliy](https://github.com/chaliy)
- feat: plugins subsystem — marketplaces and installed plugins ([#2150](https://github.com/everruns/everruns/pull/2150)) by [@chaliy](https://github.com/chaliy)
- feat(worker): gRPC session task reaper and stale-attempt fencing ([#2152](https://github.com/everruns/everruns/pull/2152)) by [@chaliy](https://github.com/chaliy)
- fix(memory): enforce file API RBAC with RFC 9457-compliant errors ([#2128](https://github.com/everruns/everruns/pull/2128)) by [@chaliy](https://github.com/chaliy)
- fix(session-tasks): enforce command policies ([#2127](https://github.com/everruns/everruns/pull/2127)) by [@chaliy](https://github.com/chaliy)
- fix(voice): redact provider error bodies ([#2105](https://github.com/everruns/everruns/pull/2105)) by [@chaliy](https://github.com/chaliy)
- fix(core): prune background session permits ([#2104](https://github.com/everruns/everruns/pull/2104)) by [@chaliy](https://github.com/chaliy)
- fix(core): guard OpenAI tool call DONE fallback ([#2134](https://github.com/everruns/everruns/pull/2134)) by [@chaliy](https://github.com/chaliy)
- fix(lua): avoid multibyte catalog truncation panic ([#2133](https://github.com/everruns/everruns/pull/2133)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add tool_output_distillation capability ([#2148](https://github.com/everruns/everruns/pull/2148)) by [@chaliy](https://github.com/chaliy)
- feat(core): name-weighted tool_search ranking with top-band reveal ([#2146](https://github.com/everruns/everruns/pull/2146)) by [@chaliy](https://github.com/chaliy)
- feat: retire session_resources dual-write for work kinds ([#2143](https://github.com/everruns/everruns/pull/2143)) by [@chaliy](https://github.com/chaliy)

## [0.11.0] - 2026-06-12

### Highlights

- **Declarative Capability Editor** - New full-page UI for managing MCP servers, skills, and files in one place ([#2141](https://github.com/everruns/everruns/pull/2141)).
- **Web Fetch via Egress + Network Access UI** - `web_fetch` is now routed through the egress service with a network access controls panel in the UI ([#2118](https://github.com/everruns/everruns/pull/2118)).
- **Session Tasks** - Background task registry with cancellation, wake policy, reaper, and chat chips for visibility ([#2119](https://github.com/everruns/everruns/pull/2119)).
- **Memory API** - Memory file CRUD HTTP API; "Workspace Volume" renamed to "Memory" throughout ([#2111](https://github.com/everruns/everruns/pull/2111), [#2106](https://github.com/everruns/everruns/pull/2106)).
- **Subagent Durable Reattach** - Durable spawn handles let long-running subagents reconnect after interruption (EVE-535).
- **OpenRouter Routing Controls** - Fine-grained model routing configuration for OpenRouter ([#2123](https://github.com/everruns/everruns/pull/2123)).

### What's Changed

- feat(ui): full-page declarative capability editor with MCP, skills, files UI ([#2141](https://github.com/everruns/everruns/pull/2141)) by [@chaliy](https://github.com/chaliy)
- feat(core): fetchkit 0.4 transport injection for web_fetch egress ([#2139](https://github.com/everruns/everruns/pull/2139)) by [@chaliy](https://github.com/chaliy)
- perf(runtime-mcp): concurrent + cached per-session tool discovery ([#2138](https://github.com/everruns/everruns/pull/2138)) by [@chaliy](https://github.com/chaliy)
- feat(core): progressive disclosure + never-defer for tool_search ([#2130](https://github.com/everruns/everruns/pull/2130)) by [@chaliy](https://github.com/chaliy)
- feat(harness): enable message_metadata on generic harness ([#2136](https://github.com/everruns/everruns/pull/2136)) by [@chaliy](https://github.com/chaliy)
- perf(mcp): stale-while-revalidate + single-flight tool cache ([#2131](https://github.com/everruns/everruns/pull/2131)) by [@chaliy](https://github.com/chaliy)
- feat: monitor session task kind ([#2129](https://github.com/everruns/everruns/pull/2129)) by [@chaliy](https://github.com/chaliy)
- feat(llm): add OpenRouter routing controls ([#2123](https://github.com/everruns/everruns/pull/2123)) by [@chaliy](https://github.com/chaliy)
- fix(worker): handle non-terminal subagent waits ([#2124](https://github.com/everruns/everruns/pull/2124)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): config schemas and localized metadata (uk) ([#2120](https://github.com/everruns/everruns/pull/2120)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): rename virtual_bash to bashkit_shell ([#2122](https://github.com/everruns/everruns/pull/2122)) by [@chaliy](https://github.com/chaliy)
- feat: session task cancellation, wake policy, reaper, and chat chips ([#2119](https://github.com/everruns/everruns/pull/2119)) by [@chaliy](https://github.com/chaliy)
- feat(core): streaming session completions for command host (EVE-543) ([#2117](https://github.com/everruns/everruns/pull/2117)) by [@chaliy](https://github.com/chaliy)
- feat: web_fetch via egress service + network access UI ([#2118](https://github.com/everruns/everruns/pull/2118)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): annotate LLM-facing messages with metadata ([#2116](https://github.com/everruns/everruns/pull/2116)) by [@chaliy](https://github.com/chaliy)
- feat(runtime): inject user connection resolver into in-process runtime ([#2115](https://github.com/everruns/everruns/pull/2115)) by [@chaliy](https://github.com/chaliy)
- fix(core): simplify web_fetch system-policy block message ([#2113](https://github.com/everruns/everruns/pull/2113)) by [@chaliy](https://github.com/chaliy)
- feat: session task registry for background work ([#2110](https://github.com/everruns/everruns/pull/2110)) by [@chaliy](https://github.com/chaliy)
- feat(memory): Memory file CRUD HTTP API ([#2111](https://github.com/everruns/everruns/pull/2111)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): /btw via extended command contract (EVE-543) ([#2109](https://github.com/everruns/everruns/pull/2109)) by [@chaliy](https://github.com/chaliy)
- feat(core): durable spawn handles and subagent reattach (EVE-535) by [@chaliy](https://github.com/chaliy)
- refactor: rename Workspace Volume to Memory; drop legacy memory_stores ([#2106](https://github.com/everruns/everruns/pull/2106)) by [@chaliy](https://github.com/chaliy)
- feat(core): partial-stream finalize on replay (EVE-532) ([#2098](https://github.com/everruns/everruns/pull/2098)) by [@chaliy](https://github.com/chaliy)
- fix(a2a): accept linked client JSON-RPC methods (EVE-540) by [@chaliy](https://github.com/chaliy)
- feat(core): transcript repair for dangling tool calls (EVE-533) by [@chaliy](https://github.com/chaliy)
- fix(worker): support subagent metadata over grpc (EVE-538) by [@chaliy](https://github.com/chaliy)
- feat(ui): custom HTTP headers in Add MCP Server form (EVE-541) ([#2107](https://github.com/everruns/everruns/pull/2107)) by [@chaliy](https://github.com/chaliy)
- fix(core): guard thinking stream output ([#2103](https://github.com/everruns/everruns/pull/2103)) by [@chaliy](https://github.com/chaliy)
- fix(egress): narrow system allowlist hosts ([#2101](https://github.com/everruns/everruns/pull/2101)) by [@chaliy](https://github.com/chaliy)
- fix(openai): restrict OpenRouter model discovery ([#2100](https://github.com/everruns/everruns/pull/2100)) by [@chaliy](https://github.com/chaliy)
- fix(bedrock): bound stream event buffering by [@chaliy](https://github.com/chaliy)
- refactor(brand): mathematically center the three-ring logo ([#2099](https://github.com/everruns/everruns/pull/2099)) by [@chaliy](https://github.com/chaliy)

## [0.10.0] - 2026-06-10

### Highlights

- **Claude Fable 5 support** - New model with adaptive thinking capability ([#2083](https://github.com/everruns/everruns/pull/2083)).
- **AWS Bedrock Runtime provider** - Run agents on any Bedrock-hosted model without leaving your AWS environment ([#2080](https://github.com/everruns/everruns/pull/2080)).
- **OpenRouter provider** - Route to hundreds of models through a single OpenRouter integration.
- **Sandboxed Lua execution** - Experimental capability to run tool logic in an isolated Lua sandbox ([#2078](https://github.com/everruns/everruns/pull/2078)).
- **System-wide outbound allowlist** - Operators can now restrict all agent egress to an explicit allowlist ([#2088](https://github.com/everruns/everruns/pull/2088)).
- **Stream-liveness heartbeat** - Reason activity now sends periodic heartbeats so long-running inference never silently stalls (EVE-531).

### What's Changed

- feat(worker): stream-liveness heartbeat for Reason activity (EVE-531) by [@chaliy](https://github.com/chaliy)
- fix(seed): block env key seeding for open auth ([#2086](https://github.com/everruns/everruns/pull/2086)) by [@chaliy](https://github.com/chaliy)
- fix(lua): validate HTTP egress DNS targets by [@chaliy](https://github.com/chaliy)
- feat(durable): per-tool-call idempotency in Act activity (EVE-530) by [@chaliy](https://github.com/chaliy)
- feat(llm): split Opus/Fable into 200K and 1M context profiles ([#2089](https://github.com/everruns/everruns/pull/2089)) by [@chaliy](https://github.com/chaliy)
- feat(llm): add OpenRouter provider by [@chaliy](https://github.com/chaliy)
- fix(a2a): publish supportedInterfaces in app AgentCard by [@chaliy](https://github.com/chaliy)
- feat(egress): system-wide outbound allowlist ([#2088](https://github.com/everruns/everruns/pull/2088)) by [@chaliy](https://github.com/chaliy)
- feat(lua): route non-essential tool calls through Lua code mode ([#2087](https://github.com/everruns/everruns/pull/2087)) by [@chaliy](https://github.com/chaliy)
- fix(bedrock): drop legacy rustls 0.21 stack from AWS SDK features ([#2084](https://github.com/everruns/everruns/pull/2084)) by [@chaliy](https://github.com/chaliy)
- feat(models): Claude Fable 5 support with adaptive thinking ([#2083](https://github.com/everruns/everruns/pull/2083)) by [@chaliy](https://github.com/chaliy)
- fix(core): classify 403 model_not_found as model_unavailable by [@chaliy](https://github.com/chaliy)
- feat(bedrock): add AWS Bedrock Runtime LLM provider ([#2080](https://github.com/everruns/everruns/pull/2080)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): honor user prompt mutations ([#2074](https://github.com/everruns/everruns/pull/2074)) by [@chaliy](https://github.com/chaliy)
- feat(lua): sandboxed Lua execution capability (experimental) ([#2078](https://github.com/everruns/everruns/pull/2078)) by [@chaliy](https://github.com/chaliy)
- fix(core): disclose generic tool search stubs by [@chaliy](https://github.com/chaliy)

## [0.9.0] - 2026-06-05

### Highlights

- **Model-adaptive `auto_tool_search`** - Uses OpenAI's hosted `tool_search` on supported models and the generic client-side fallback everywhere else; now the default in the `generic` harness ([#2056](https://github.com/everruns/everruns/pull/2056)).
- **Deferred MCP tool schemas** - `tool_search` now defers MCP tool schema loading for any generic MCP server, reducing context overhead on first turn (EVE-524).
- **Per-generation cost tracking** - Actual and estimated cost captured per-generation with denormalized totals for accurate billing attribution ([#2060](https://github.com/everruns/everruns/pull/2060)).
- **OpenRouter enhancements** - Reasoning capability discovery ([#2063](https://github.com/everruns/everruns/pull/2063)) and enriched model sync with profiles and vendor icons ([#2067](https://github.com/everruns/everruns/pull/2067)).
- **Flat model registry** - Vendor tags and surface predicates for cleaner model resolution ([#2066](https://github.com/everruns/everruns/pull/2066)).

### What's Changed

- feat(capabilities): model-adaptive `auto_tool_search` dispatcher ([#2056](https://github.com/everruns/everruns/pull/2056)) by [@chaliy](https://github.com/chaliy)
- test(llm): verify auto_tool_search hosted round-trip on GPT-5.4 ([#2058](https://github.com/everruns/everruns/pull/2058)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): align tool search capability name by [@chaliy](https://github.com/chaliy)
- feat(models): add profiles and vendor icons for latest flagship models ([#2057](https://github.com/everruns/everruns/pull/2057)) by [@chaliy](https://github.com/chaliy)
- feat(usage): per-generation actual/estimated cost + denormalized totals ([#2060](https://github.com/everruns/everruns/pull/2060)) by [@chaliy](https://github.com/chaliy)
- fix(llm): support hosted OpenAI tool search by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): use is_local() instead of hard-coded Stdio check ([#2055](https://github.com/everruns/everruns/pull/2055)) by [@chaliy](https://github.com/chaliy)
- fix(tools): atomic per-session background run cap via in-process semaphore ([#2061](https://github.com/everruns/everruns/pull/2061)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): honor resolve_for_model in all collection paths ([#2064](https://github.com/everruns/everruns/pull/2064)) by [@chaliy](https://github.com/chaliy)
- feat(openai): discover OpenRouter reasoning capability ([#2063](https://github.com/everruns/everruns/pull/2063)) by [@chaliy](https://github.com/chaliy)
- feat(tool-search): defer MCP tool schemas under generic tool_search (EVE-524) by [@chaliy](https://github.com/chaliy)
- refactor(models): flat model registry with vendor tags and surface predicates ([#2066](https://github.com/everruns/everruns/pull/2066)) by [@chaliy](https://github.com/chaliy)
- feat(models): enrich OpenRouter model sync ([#2067](https://github.com/everruns/everruns/pull/2067)) by [@chaliy](https://github.com/chaliy)
- fix(core): scope tool_search registry introspection by [@chaliy](https://github.com/chaliy)
- fix(ci): enforce msrv in aggregate gate ([#2073](https://github.com/everruns/everruns/pull/2073)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): enforce quotas on copy and CAS writes ([#2068](https://github.com/everruns/everruns/pull/2068)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve stateful OpenAI tool outputs ([#2072](https://github.com/everruns/everruns/pull/2072)) by [@chaliy](https://github.com/chaliy)
- fix(core): abort cpu-bound tool tasks on cancellation ([#2069](https://github.com/everruns/everruns/pull/2069)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): honor auto tool search threshold ([#2071](https://github.com/everruns/everruns/pull/2071)) by [@chaliy](https://github.com/chaliy)

## [0.8.38] - 2026-06-05

### Highlights

- **Generic `tool_search` capability** - Provider-agnostic deferred tool loading now works on any model (Anthropic, Gemini, and others); MCP server tools are first-class `ToolRegistry` tools across all hosts via `McpProxyTool`/`McpToolInvoker`, so `tool_search`, `openai_tool_search`, and `spawn_background` work transparently with MCP tools ([#2050](https://github.com/everruns/everruns/pull/2050)).
- **Pre-tool-use hook seam** - New `pre_tool_use_hooks` capability seam enables fine-grained tool gating before execution ([#2048](https://github.com/everruns/everruns/pull/2048)).

### What's Changed

- fix(core): emit tool calls when finish chunk carries empty content (EVE-522) ([#2052](https://github.com/everruns/everruns/pull/2052)) by [@chaliy](https://github.com/chaliy)
- fix(core): replay full transcript on stateless Responses gateways ([#2051](https://github.com/everruns/everruns/pull/2051)) by [@chaliy](https://github.com/chaliy)
- feat: generic tool_search + first-class MCP tools across all hosts ([#2050](https://github.com/everruns/everruns/pull/2050)) by [@chaliy](https://github.com/chaliy)
- fix(llm): gate tool_search off on gpt-5.5 family (EVE-521) ([#2049](https://github.com/everruns/everruns/pull/2049)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add pre_tool_use_hooks seam for tool gating ([#2048](https://github.com/everruns/everruns/pull/2048)) by [@chaliy](https://github.com/chaliy)
- refactor(payments): move Parallel paid capability out of core ([#2047](https://github.com/everruns/everruns/pull/2047)) by [@chaliy](https://github.com/chaliy)
## [0.8.37] - 2026-06-03

### Highlights

- **MCP client in the runtime** - New `everruns-mcp` crate ships a first-class MCP client with stdio transport, wired into the runtime and coding-CLI ([#2045](https://github.com/everruns/everruns/pull/2045)).
- **API keys renamed to personal access tokens** - User-scoped auth credentials are now consistently called "personal access tokens" across the table (`personal_access_tokens`), API (`/v1/auth/personal-access-tokens`), UI (Settings > Personal access tokens), CLI, specs, and docs. Tokens are now prefixed `evr_pat_` instead of `evr_`; existing tokens are invalidated and must be re-created (re-run `everruns login`) ([#2043](https://github.com/everruns/everruns/pull/2043)).
- **Security hardening** - SSRF DNS pinning for MCP server execution (EVE-516), ReDoS hardening for grep-based tool call regex (EVE-517), distributed + per-account rate limiting (EVE-513), and fail-closed LLM key resolution with env fallback removed (EVE-511).
- **Org-level controls** - Feature flags with opt-in UI and API, per-org soft caps on concurrent sessions and active turns (EVE-508), per-org outbound tool-call rate limiting, and concurrency/volume caps for eval runs (EVE-509).
- **User-defined hooks** - Composable bash executor for lifecycle hooks; `user_prompt_submit` and `turn_end` events now available ([#2022](https://github.com/everruns/everruns/pull/2022)).
- **Session file quotas** - Per-file and per-session byte quotas enforced (EVE-510).

### What's Changed

- feat(mcp): MCP client in the runtime (everruns-mcp crate, stdio, coding-CLI) ([#2045](https://github.com/everruns/everruns/pull/2045)) by [@chaliy](https://github.com/chaliy)
- fix(deno): retry connect_sandbox on 404 DEPLOYMENT_NOT_FOUND ([#2044](https://github.com/everruns/everruns/pull/2044)) by [@chaliy](https://github.com/chaliy)
- refactor(auth): rename API keys to personal access tokens ([#2043](https://github.com/everruns/everruns/pull/2043)) by [@chaliy](https://github.com/chaliy)
- feat(seed): materialize DEFAULT_*_API_KEY for single-tenant/dev ([#2042](https://github.com/everruns/everruns/pull/2042)) by [@chaliy](https://github.com/chaliy)
- feat(feature-flags): org-level feature flag opt-in UI and API by [@chaliy](https://github.com/chaliy)
- feat(sessions): add per-org soft cap on concurrent sessions and active turns (EVE-508) by [@chaliy](https://github.com/chaliy)
- docs(user-hooks): add user_prompt_submit and turn_end examples by [@chaliy](https://github.com/chaliy)
- fix(runtime): keep OpenAI tool call/result pairs during history trimming (EVE-519) by [@chaliy](https://github.com/chaliy)
- feat(evals): concurrency and volume caps for eval runs (EVE-509) by [@chaliy](https://github.com/chaliy)
- feat(session-files): enforce per-file and per-session byte quotas (EVE-510) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): wire the four remaining user-hook lifecycle events ([#2032](https://github.com/everruns/everruns/pull/2032)) by [@chaliy](https://github.com/chaliy)
- feat(security): SSRF DNS pinning for MCP server execution (EVE-516) by [@chaliy](https://github.com/chaliy)
- fix(security): harden grep-based tool call regex against ReDoS (EVE-517) by [@chaliy](https://github.com/chaliy)
- feat(e2b,deno): BYO-only sandbox credentials (EVE-505) ([#2033](https://github.com/everruns/everruns/pull/2033)) by [@chaliy](https://github.com/chaliy)
- feat(rate-limit): per-org outbound tool-call rate limiting (TM-TOOL-009) by [@chaliy](https://github.com/chaliy)
- feat(apps): schedule channel rate limits (EVE-507) by [@chaliy](https://github.com/chaliy)
- feat(feature-flags): agent_delegation feature flag (EVE-506) by [@chaliy](https://github.com/chaliy)
- feat(security): distributed + per-account rate limiting (EVE-513) by [@chaliy](https://github.com/chaliy)
- feat(llm): fail-closed key resolution, remove env fallback (EVE-511) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): user-defined hooks via composable bash executor ([#2022](https://github.com/everruns/everruns/pull/2022)) by [@chaliy](https://github.com/chaliy)
- feat(plugin): add production everruns plugin by [@chaliy](https://github.com/chaliy)
- feat(core): class-aware tool execution scheduler for ActAtom ([#2020](https://github.com/everruns/everruns/pull/2020)) by [@chaliy](https://github.com/chaliy)
- feat(ui): smooth chat streaming text by [@chaliy](https://github.com/chaliy)
- test(runtime): end-to-end ActAtom scheduler integration tests ([#2021](https://github.com/everruns/everruns/pull/2021)) by [@chaliy](https://github.com/chaliy)
- fix(openapi): preserve enum refs for schema examples ([#2014](https://github.com/everruns/everruns/pull/2014)) by [@chaliy](https://github.com/chaliy)
- fix(events): correct SSE id cursor and voice event docs ([#2012](https://github.com/everruns/everruns/pull/2012)) by [@chaliy](https://github.com/chaliy)
- fix(api): fail-close llm metadata defaults ([#2008](https://github.com/everruns/everruns/pull/2008)) by [@chaliy](https://github.com/chaliy)
- fix(voice): bridge realtime speech to durable chat by [@chaliy](https://github.com/chaliy)
- fix(ui): hide work log for direct answers by [@chaliy](https://github.com/chaliy)
- fix(mcp): extract JSON from SSE response body on tools fetch by [@chaliy](https://github.com/chaliy)
- fix(core): disable direct egress redirect following by [@chaliy](https://github.com/chaliy)
- fix(api): align session cancel action with command behavior by [@chaliy](https://github.com/chaliy)
- fix(api): delegate allowed_actions from ResourceWithCounts by [@chaliy](https://github.com/chaliy)
- fix(api): correct cancel turn OpenAPI success example by [@chaliy](https://github.com/chaliy)
- fix(core): avoid unwrapping user _raw_output_scalar keys by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): replay reason.item signatures on resume by [@chaliy](https://github.com/chaliy)
- fix(cli): clarify /clear command scope in description by [@chaliy](https://github.com/chaliy)
- fix(plugin): use neutral Codex marketplace label by [@chaliy](https://github.com/chaliy)
- chore(specs): resolve TM-TENANT-008 — GET /v1/users already org-scoped (EVE-515) by [@chaliy](https://github.com/chaliy)
- chore(rust): upgrade toolchain to 1.96 by [@chaliy](https://github.com/chaliy)

## [0.8.36] - 2026-05-29

### Highlights

- **Hypermedia entity actions across resources** - Foundation for entity-level HATEOAS actions ([#1986](https://github.com/everruns/everruns/pull/1986)), now rolled out to Agents, Harnesses, Apps, and Skills ([#1989](https://github.com/everruns/everruns/pull/1989)).
- **MCP traffic governed by EgressService** - MCP client calls now route through the dedicated outbound egress service ([#1993](https://github.com/everruns/everruns/pull/1993)), and MCP execute returns a structured-error envelope ([#1987](https://github.com/everruns/everruns/pull/1987)).
- **OpenAPI extensions and example coverage** - LLM-specific OpenAPI extension foundation (`x-llm-*`) ([#1991](https://github.com/everruns/everruns/pull/1991)), SSE event-type catalog for the session stream ([#1990](https://github.com/everruns/everruns/pull/1990)), per-operation request/response example pairs foundation ([#1992](https://github.com/everruns/everruns/pull/1992)), and field-example coverage 37% → 44% ([#1988](https://github.com/everruns/everruns/pull/1988)).
- **background_execution capability surfaces spawn_background** - Capabilities can now expose `spawn_background` through `background_execution` ([#1996](https://github.com/everruns/everruns/pull/1996)).
- **ERcode persists reasoning artifacts** - Reasoning artifacts are persisted and rendered on session restore ([#1985](https://github.com/everruns/everruns/pull/1985)).

### What's Changed

- fix(ui): route sidebar Profile menu to /settings/profile ([#2001](https://github.com/everruns/everruns/pull/2001)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit 0.7.2 -> 0.8.0 ([#2000](https://github.com/everruns/everruns/pull/2000)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump OTEL 0.32, async-nats 0.49, sqlx 0.9, rusqlite 0.39 ([#1998](https://github.com/everruns/everruns/pull/1998)) by [@chaliy](https://github.com/chaliy)
- feat(core): add scripted llmsim responses by [@chaliy](https://github.com/chaliy)
- fix(ui): interpolate duration in chat turn divider ([#1997](https://github.com/everruns/everruns/pull/1997)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): expose spawn_background via background_execution ([#1996](https://github.com/everruns/everruns/pull/1996)) by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): confine output chmod walk to /outputs subtree ([#1994](https://github.com/everruns/everruns/pull/1994)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump llmsim from 0.3.0 to 0.4.0 ([#1995](https://github.com/everruns/everruns/pull/1995)) by [@chaliy](https://github.com/chaliy)
- docs(api): per-operation request/response example pairs (foundation) ([#1992](https://github.com/everruns/everruns/pull/1992)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): route MCP client traffic through EgressService ([#1993](https://github.com/everruns/everruns/pull/1993)) by [@chaliy](https://github.com/chaliy)
- feat(api): LLM-specific OpenAPI extensions foundation (x-llm-*) ([#1991](https://github.com/everruns/everruns/pull/1991)) by [@chaliy](https://github.com/chaliy)
- docs(api): SSE event-type catalog in OpenAPI (session stream) ([#1990](https://github.com/everruns/everruns/pull/1990)) by [@chaliy](https://github.com/chaliy)
- feat(api): roll out hypermedia entity actions to Agents/Harnesses/Apps/Skills ([#1989](https://github.com/everruns/everruns/pull/1989)) by [@chaliy](https://github.com/chaliy)
- feat(api): structured-error envelope for MCP execute (server-side) ([#1987](https://github.com/everruns/everruns/pull/1987)) by [@chaliy](https://github.com/chaliy)
- docs(api): push field-example coverage 37% → 44% (operator + LLM model wave) ([#1988](https://github.com/everruns/everruns/pull/1988)) by [@chaliy](https://github.com/chaliy)
- feat(api): hypermedia entity actions foundation, sessions pilot ([#1986](https://github.com/everruns/everruns/pull/1986)) by [@chaliy](https://github.com/chaliy)
- feat(ercode): persist and render reasoning artifacts for session restore ([#1985](https://github.com/everruns/everruns/pull/1985)) by [@chaliy](https://github.com/chaliy)

## [0.8.35] - 2026-05-27

### Highlights

- **Outbound egress service** - Core gains a dedicated outbound egress service for governing external network calls from runtime components.
- **Auto output mode for exec tools** - Exec tools default to a smarter `auto` output mode that adapts presentation to result size ([#1961](https://github.com/everruns/everruns/pull/1961)).
- **Security hardening across CI and tokens** - Fork PRs can no longer read Doppler or LLM keys via env ([#1970](https://github.com/everruns/everruns/pull/1970)), the GitHub integration blocks implicit parent-token fallback ([#1966](https://github.com/everruns/everruns/pull/1966)), and the coding CLI tightens session output artifact permissions ([#1967](https://github.com/everruns/everruns/pull/1967)).
- **OpenAPI documentation completeness** - Field-example coverage climbed from 24% to 37% ([#1980](https://github.com/everruns/everruns/pull/1980)) and Reporting/Payment/Voice/list-query schemas reached a 99% description floor ([#1972](https://github.com/everruns/everruns/pull/1972)).

### What's Changed

- chore(deps): bump bashkit to 0.7.2 ([#1982](https://github.com/everruns/everruns/pull/1982)) by [@chaliy](https://github.com/chaliy)
- chore(docs): switch docs app to pnpm isolated linker ([#1953](https://github.com/everruns/everruns/pull/1953)) by [@chaliy](https://github.com/chaliy)
- fix(core): harden edit_file placeholders by [@chaliy](https://github.com/chaliy)
- docs(api): field-example coverage 24% -> 37% (grand finale) ([#1980](https://github.com/everruns/everruns/pull/1980)) by [@chaliy](https://github.com/chaliy)
- docs(api): describe Reporting/Payment/Voice/list-query schemas; floor 99% ([#1972](https://github.com/everruns/everruns/pull/1972)) by [@chaliy](https://github.com/chaliy)
- fix(core): type-dispatch Event and EventRequest deserialization ([#1979](https://github.com/everruns/everruns/pull/1979)) by [@chaliy](https://github.com/chaliy)
- fix(core): cap oversized native tool images ([#1976](https://github.com/everruns/everruns/pull/1976)) by [@chaliy](https://github.com/chaliy)
- fix(container-sandbox): persist raw output in auto success ([#1977](https://github.com/everruns/everruns/pull/1977)) by [@chaliy](https://github.com/chaliy)
- fix(api): correct schedule execution status example ([#1968](https://github.com/everruns/everruns/pull/1968)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove deleted .npmrc from Docker build inputs ([#1978](https://github.com/everruns/everruns/pull/1978)) by [@chaliy](https://github.com/chaliy)
- feat(core): add outbound egress service by [@chaliy](https://github.com/chaliy)
- docs: add physical-architecture page covering deployable components ([#1973](https://github.com/everruns/everruns/pull/1973)) by [@chaliy](https://github.com/chaliy)
- chore(skills): mark internal repo skills by [@chaliy](https://github.com/chaliy)
- fix(ci): stop fork PRs from reading Doppler + LLM keys via env ([#1970](https://github.com/everruns/everruns/pull/1970)) by [@chaliy](https://github.com/chaliy)
- docs(api): describe Voice/Report/GitDiff fields; floor 92% ([#1965](https://github.com/everruns/everruns/pull/1965)) by [@chaliy](https://github.com/chaliy)
- docs: remove internal spec references from public docs ([#1969](https://github.com/everruns/everruns/pull/1969)) by [@chaliy](https://github.com/chaliy)
- chore(deps): enforce pnpm release age floor by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): secure session output artifact permissions ([#1967](https://github.com/everruns/everruns/pull/1967)) by [@chaliy](https://github.com/chaliy)
- fix(github): block implicit parent token fallback ([#1966](https://github.com/everruns/everruns/pull/1966)) by [@chaliy](https://github.com/chaliy)
- ci(release): use homebrew tap token by [@chaliy](https://github.com/chaliy)
- feat(api): examples on payments, reports, voice, misc; floor 24% ([#1958](https://github.com/everruns/everruns/pull/1958)) by [@chaliy](https://github.com/chaliy)
- fix(tests): make Daytona /sandbox listing assertion best-effort ([#1963](https://github.com/everruns/everruns/pull/1963)) by [@chaliy](https://github.com/chaliy)
- fix(tests): tolerate Daytona /sandbox pagination shape ([#1962](https://github.com/everruns/everruns/pull/1962)) by [@chaliy](https://github.com/chaliy)
- feat(tools): add auto output mode and make it default for exec ([#1961](https://github.com/everruns/everruns/pull/1961)) by [@chaliy](https://github.com/chaliy)
- fix(llm): avoid mixing previous_response_id with full transcript ([#1960](https://github.com/everruns/everruns/pull/1960)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump starlight-openapi from 0.24.0 to 0.25.3 in /apps/docs ([#1937](https://github.com/everruns/everruns/pull/1937)) by @dependabot
- chore(deps-dev): bump typescript from 5.9.3 to 6.0.3 in /apps/ui ([#1940](https://github.com/everruns/everruns/pull/1940)) by @dependabot
- chore(deps-dev): bump @ag-ui/core from 0.0.45 to 0.0.53 in /apps/ui ([#1938](https://github.com/everruns/everruns/pull/1938)) by @dependabot
- chore(deps): bump astro from 6.3.6 to 6.3.7 in /apps/docs ([#1936](https://github.com/everruns/everruns/pull/1936)) by @dependabot
- chore(deps-dev): bump oxfmt from 0.48.0 to 0.51.0 in /apps/ui ([#1939](https://github.com/everruns/everruns/pull/1939)) by @dependabot
- fix(llm): stabilize prompt cache requests by [@chaliy](https://github.com/chaliy)
- chore(examples): upgrade ercode similar dependency by [@chaliy](https://github.com/chaliy)

## [0.8.34] - 2026-05-24

### Highlights

- **Coding-cli supports OpenRouter and Ollama** - The embedded coding CLI now talks to OpenRouter and Ollama in addition to the existing providers, broadening local and BYO-model setups.
- **Capability-provided slash commands in the embedded CLI** - Capabilities can register their own slash commands inside the embedded coding CLI ([#1903](https://github.com/everruns/everruns/pull/1903)).
- **Fingerprint-based loop detection** - Runtime now detects repeating-output loops via fingerprints and breaks them automatically.
- **Stale tool result masking for cost control** - Compaction masks stale tool results so long sessions stay within budget without losing recent context.
- **GitHub Scout blueprint** - New built-in GitHub Scout blueprint plus a fallback to the parent session token when the scout runs ([#1951](https://github.com/everruns/everruns/pull/1951)).
- **Hypermedia pagination links** - Paginated API responses now include `next_url` / `prev_url` for HATEOAS-style navigation ([#1889](https://github.com/everruns/everruns/pull/1889)).
- **OpenAPI examples expansion** - Examples and field descriptions added across Create*/Update* schemas, Schedules, A2A channel, volume sources, Memory + Knowledge clusters, and file/git/durable/tool-results endpoints; field-example floor ratcheted from 15% to 21% ([#1904](https://github.com/everruns/everruns/pull/1904), [#1915](https://github.com/everruns/everruns/pull/1915), [#1923](https://github.com/everruns/everruns/pull/1923), [#1949](https://github.com/everruns/everruns/pull/1949), [#1952](https://github.com/everruns/everruns/pull/1952)).

### What's Changed

- feat(api): examples on file ops, git, durable, tool-results; floor 21% ([#1952](https://github.com/everruns/everruns/pull/1952)) by [@chaliy](https://github.com/chaliy)
- chore(ui): adopt Turbopack and drop hoisted-linker custom ([#1950](https://github.com/everruns/everruns/pull/1950)) by [@chaliy](https://github.com/chaliy)
- fix(github): fallback to parent session token for scout ([#1951](https://github.com/everruns/everruns/pull/1951)) by [@chaliy](https://github.com/chaliy)
- fix(ui): clean up jsx-a11y and react lint warnings ([#1955](https://github.com/everruns/everruns/pull/1955)) by [@chaliy](https://github.com/chaliy)
- fix(ci): restore docker build triggers for rust inputs ([#1933](https://github.com/everruns/everruns/pull/1933)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve CAS semantics in approval-gated writes ([#1930](https://github.com/everruns/everruns/pull/1930)) by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): bound write_todos transcript rendering ([#1932](https://github.com/everruns/everruns/pull/1932)) by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): redact credentials in git remote context ([#1931](https://github.com/everruns/everruns/pull/1931)) by [@chaliy](https://github.com/chaliy)
- fix(core): bound infinity candidate load with max cap ([#1929](https://github.com/everruns/everruns/pull/1929)) by [@chaliy](https://github.com/chaliy)
- fix(core): tighten token-efficient tool outputs ([#1954](https://github.com/everruns/everruns/pull/1954)) by [@chaliy](https://github.com/chaliy)
- feat(events): persist opaque assistant reasoning items ([#1948](https://github.com/everruns/everruns/pull/1948)) by [@chaliy](https://github.com/chaliy)
- feat(api): examples on Memory + Knowledge clusters; floor 19% ([#1949](https://github.com/everruns/everruns/pull/1949)) by [@chaliy](https://github.com/chaliy)
- feat(ercode): store session artifacts in folders by [@chaliy](https://github.com/chaliy)
- fix(core): keep model view masking capability-owned ([#1945](https://github.com/everruns/everruns/pull/1945)) by [@chaliy](https://github.com/chaliy)
- feat(coding-cli): support OpenRouter and Ollama by [@chaliy](https://github.com/chaliy)
- feat(context): attribute session usage sources by [@chaliy](https://github.com/chaliy)
- feat(core): bound prompt tool output by default ([#1943](https://github.com/everruns/everruns/pull/1943)) by [@chaliy](https://github.com/chaliy)
- feat(core): add fingerprint loop detection by [@chaliy](https://github.com/chaliy)
- fix(cli): group consecutive tool output rows by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): support multiline TUI input by [@chaliy](https://github.com/chaliy)
- chore(ui,docs): adopt pnpm for UI dev across all environments ([#1927](https://github.com/everruns/everruns/pull/1927)) by [@chaliy](https://github.com/chaliy)
- feat(compaction): mask stale tool results for cost control by [@chaliy](https://github.com/chaliy)
- feat(core): add utility llm service by [@chaliy](https://github.com/chaliy)
- feat(api): examples on Schedules, A2A channel, volume sources; floor 17% ([#1923](https://github.com/everruns/everruns/pull/1923)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): avoid lossy in-process org id folding ([#1924](https://github.com/everruns/everruns/pull/1924)) by [@chaliy](https://github.com/chaliy)
- fix(agents): bound automatic snapshots by [@chaliy](https://github.com/chaliy)
- feat(github): add github scout blueprint by [@chaliy](https://github.com/chaliy)
- perf(prompts): trim remaining capability prompts by [@chaliy](https://github.com/chaliy)
- docs(crates): prepare published packages ([#1919](https://github.com/everruns/everruns/pull/1919)) by [@chaliy](https://github.com/chaliy)
- fix(agent-handoff): require explicit target harness for child session ([#1921](https://github.com/everruns/everruns/pull/1921)) by [@chaliy](https://github.com/chaliy)
- perf(prompts): trim hot capability and harness prompts ([#1913](https://github.com/everruns/everruns/pull/1913)) by [@chaliy](https://github.com/chaliy)
- feat(api): examples on Update* request schemas; status enums; floor 15% ([#1915](https://github.com/everruns/everruns/pull/1915)) by [@chaliy](https://github.com/chaliy)
- docs(capabilities): order overview first by [@chaliy](https://github.com/chaliy)
- feat(events): summarize completed turns ([#1918](https://github.com/everruns/everruns/pull/1918)) by [@chaliy](https://github.com/chaliy)
- fix(events): persist llm generation events ([#1917](https://github.com/everruns/everruns/pull/1917)) by [@chaliy](https://github.com/chaliy)
- feat(ercode): inject environment context by [@chaliy](https://github.com/chaliy)
- chore(deps): bump distroless/cc-debian12 from `e2d29ae` to `bd2899c` in /crates/server ([#1900](https://github.com/everruns/everruns/pull/1900)) by [@dependabot](https://github.com/dependabot)
- fix(cli): hide assistant thinking in session logs by [@chaliy](https://github.com/chaliy)
- fix(runtime): derive in-process org id from session org ([#1890](https://github.com/everruns/everruns/pull/1890)) by [@chaliy](https://github.com/chaliy)
- fix(core): trim infinity context by token budget by [@chaliy](https://github.com/chaliy)
- ci: filter postgres integration tests by [@chaliy](https://github.com/chaliy)
- feat(api): examples on high-traffic Create*/Update* schemas; field-example gate ([#1904](https://github.com/everruns/everruns/pull/1904)) by [@chaliy](https://github.com/chaliy)
- feat(coding-cli): persist bash output by [@chaliy](https://github.com/chaliy)
- fix(coding-cli): tighten todo rendering by [@chaliy](https://github.com/chaliy)
- chore(deps): bump distroless/cc-debian12 from `e2d29ae` to `bd2899c` in /crates/worker ([#1901](https://github.com/everruns/everruns/pull/1901)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump distroless/cc-debian12 from `e2d29ae` to `bd2899c` in /docker ([#1902](https://github.com/everruns/everruns/pull/1902)) by [@dependabot](https://github.com/dependabot)
- refactor(examples): move coding CLI capabilities by [@chaliy](https://github.com/chaliy)
- refactor(coding-cli): split runtime wiring result by [@chaliy](https://github.com/chaliy)
- chore(docs): slim agent instructions by [@chaliy](https://github.com/chaliy)
- feat(coding-cli): improve turn progress rendering by [@chaliy](https://github.com/chaliy)
- feat(core,runtime,examples): capability-provided slash commands in the embedded CLI ([#1903](https://github.com/everruns/everruns/pull/1903)) by [@chaliy](https://github.com/chaliy)
- test(api): count utoipa oneOf-nested descriptions; ratchet field floor to 85% ([#1898](https://github.com/everruns/everruns/pull/1898)) by [@chaliy](https://github.com/chaliy)
- feat(coding-cli): show live turn progress by [@chaliy](https://github.com/chaliy)
- feat(api): hypermedia next_url / prev_url on paginated responses ([#1889](https://github.com/everruns/everruns/pull/1889)) by [@chaliy](https://github.com/chaliy)
- feat(examples): coding-cli — seed event collector on resume ([#1896](https://github.com/everruns/everruns/pull/1896)) by [@chaliy](https://github.com/chaliy)
- ci: bump actions/checkout to v5 (Node 24) ([#1893](https://github.com/everruns/everruns/pull/1893)) by [@chaliy](https://github.com/chaliy)
- feat(runtime): SingleSessionBuilder::session_id setter ([#1894](https://github.com/everruns/everruns/pull/1894)) by [@chaliy](https://github.com/chaliy)
- ci(publish-crates): allow manual dispatch + retry on rate limit ([#1895](https://github.com/everruns/everruns/pull/1895)) by [@chaliy](https://github.com/chaliy)

## [0.8.33] - 2026-05-19

### What's Changed

- chore(release): align `crates/core/Cargo.toml` path-dep version pins (`everruns-config`, `everruns-openui`, `everruns-a2ui`) with the workspace version so the crates-publish workflow accepts the release tag. Add `scripts/sync-publish-pin-versions.py` helper and wire it into the prepare-release flow so future releases stay in sync. By [@chaliy](https://github.com/chaliy)

## [0.8.32] - 2026-05-19

### Highlights

- **Reporting product surface** - Completes the async reporting subsystem with finalized analytics, saved reports, and export flows.
- **Agent versions with automatic draft snapshots** - Immutable agent configuration snapshots are now captured automatically as drafts to enable audit, rollback, forks, and pinned App deployments.
- **RFC 9457 Problem Details API errors** - HTTP error responses now follow RFC 9457 with structured `CommandError` extensions ([#1834](https://github.com/everruns/everruns/pull/1834)).
- **Coding-cli example** - New TUI coding agent example built on `everruns-runtime` with JSONL session log, `--session` resume, and proper harness system prompt ([#1839](https://github.com/everruns/everruns/pull/1839), [#1870](https://github.com/everruns/everruns/pull/1870), [#1871](https://github.com/everruns/everruns/pull/1871)).
- **Runtime FileSystem policy decorators** - Filesystem access in the runtime now ships with composable policy decorators for ACL enforcement and pluggable real-disk `SessionFileStore` (EVE-478) ([#1857](https://github.com/everruns/everruns/pull/1857), [#1886](https://github.com/everruns/everruns/pull/1886)).
- **OpenAPI surface expansion** - Health, agent versions, skills, schedules ([#1873](https://github.com/everruns/everruns/pull/1873)), and payments/LLM/durable field descriptions ([#1851](https://github.com/everruns/everruns/pull/1851)) are now documented in the OpenAPI spec.

### What's Changed

- feat(examples): coding-cli — JSONL session log + --session resume ([#1871](https://github.com/everruns/everruns/pull/1871)) by [@chaliy](https://github.com/chaliy)
- feat(examples): coding-cli — proper system prompt for the harness ([#1870](https://github.com/everruns/everruns/pull/1870)) by [@chaliy](https://github.com/chaliy)
- chore(ci): add affected CI opt-out labels ([#1887](https://github.com/everruns/everruns/pull/1887)) by [@chaliy](https://github.com/chaliy)
- chore(test-cases): ignore manual test results by [@chaliy](https://github.com/chaliy)
- feat(runtime): ship FileSystem policy decorators (EVE-478) ([#1886](https://github.com/everruns/everruns/pull/1886)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): reject symlink workspace paths ([#1876](https://github.com/everruns/everruns/pull/1876)) by [@chaliy](https://github.com/chaliy)
- fix(core): pin fetchkit to 0.2.0 to enforce egress ACL ([#1881](https://github.com/everruns/everruns/pull/1881)) by [@chaliy](https://github.com/chaliy)
- fix(server): harden app run history window parsing ([#1878](https://github.com/everruns/everruns/pull/1878)) by [@chaliy](https://github.com/chaliy)
- fix(openai): cap prompt cache key length by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve ACLs in single-session builder ([#1877](https://github.com/everruns/everruns/pull/1877)) by [@chaliy](https://github.com/chaliy)
- chore(deps): remove unused integration dependencies by [@chaliy](https://github.com/chaliy)
- feat(agents): add authenticated agent handoff by [@chaliy](https://github.com/chaliy)
- ci: narrow docker build trigger by [@chaliy](https://github.com/chaliy)
- refactor(runtime): resolve filesystem from platform by [@chaliy](https://github.com/chaliy)
- feat(openapi): health + agent versions + skills + schedules ([#1873](https://github.com/everruns/everruns/pull/1873)) by [@chaliy](https://github.com/chaliy)
- refactor(runtime): InProcessRuntime drives turns via host activity functions ([#1874](https://github.com/everruns/everruns/pull/1874)) by [@chaliy](https://github.com/chaliy)
- fix(ci): harden crates publish ref validation by [@chaliy](https://github.com/chaliy)
- feat(coding-cli): add model command suggestions by [@chaliy](https://github.com/chaliy)
- chore(release): publish provider crates by [@chaliy](https://github.com/chaliy)
- refactor(runtime): drop store shim wrappers, add EventBus + in_memory() ([#1872](https://github.com/everruns/everruns/pull/1872)) by [@chaliy](https://github.com/chaliy)
- feat(openapi): payments + LLM + durable field descriptions ([#1851](https://github.com/everruns/everruns/pull/1851)) by [@chaliy](https://github.com/chaliy)
- feat(api): RFC 9457 Problem Details + CommandError extensions ([#1834](https://github.com/everruns/everruns/pull/1834)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump fetchkit to 0.3.0 ([#1865](https://github.com/everruns/everruns/pull/1865)) by [@chaliy](https://github.com/chaliy)
- feat(examples): add coding-cli — TUI coding agent on everruns-runtime ([#1839](https://github.com/everruns/everruns/pull/1839)) by [@chaliy](https://github.com/chaliy)
- feat(agents): add automatic draft snapshots by [@chaliy](https://github.com/chaliy)
- chore(deps): bump brace-expansion from 5.0.5 to 5.0.6 in /.deepsec ([#1864](https://github.com/everruns/everruns/pull/1864)) by [@dependabot](https://github.com/dependabot)
- refactor(runtime): add session filesystem factory by [@chaliy](https://github.com/chaliy)
- feat(core): configure agent instruction files by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to 0.6.0 by [@chaliy](https://github.com/chaliy)
- feat(runtime): add embedder builders by [@chaliy](https://github.com/chaliy)
- feat(runtime): pluggable real-disk SessionFileStore + capability examples ([#1857](https://github.com/everruns/everruns/pull/1857)) by [@chaliy](https://github.com/chaliy)
- chore(release): publish public crates by [@chaliy](https://github.com/chaliy)
- fix(fcp): resume sessions by cookie id ([#1840](https://github.com/everruns/everruns/pull/1840)) by [@chaliy](https://github.com/chaliy)
- fix(apps): add app run history endpoint by [@chaliy](https://github.com/chaliy)
- fix(a2a): bind signing HMAC to app channel scope ([#1830](https://github.com/everruns/everruns/pull/1830)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump typescript to 6 in apps/docs ([#1855](https://github.com/everruns/everruns/pull/1855)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump marked to 18 ([#1854](https://github.com/everruns/everruns/pull/1854)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump lucide-react to 1.16 ([#1853](https://github.com/everruns/everruns/pull/1853)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove dash-gap-analysis.md ([#1852](https://github.com/everruns/everruns/pull/1852)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump tokio-tungstenite to 0.29 ([#1847](https://github.com/everruns/everruns/pull/1847)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump @openuidev/react-{lang,ui} to 0.2 / 0.11 ([#1850](https://github.com/everruns/everruns/pull/1850)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump governor to 0.10 ([#1848](https://github.com/everruns/everruns/pull/1848)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump rstest to 0.26 ([#1845](https://github.com/everruns/everruns/pull/1845)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump async-nats to 0.48 ([#1844](https://github.com/everruns/everruns/pull/1844)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump a2a stack to 0.3 ([#1842](https://github.com/everruns/everruns/pull/1842)) by [@chaliy](https://github.com/chaliy)
- chore(deps): refresh Cargo.lock within semver ([#1836](https://github.com/everruns/everruns/pull/1836)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump @astrojs/starlight to 0.39.2 ([#1837](https://github.com/everruns/everruns/pull/1837)) by [@chaliy](https://github.com/chaliy)
- chore: index 6 missing specs in AGENTS.md ([#1838](https://github.com/everruns/everruns/pull/1838)) by [@chaliy](https://github.com/chaliy)
- chore(deps): override @anthropic-ai/sdk to 0.91.1+ in .deepsec ([#1835](https://github.com/everruns/everruns/pull/1835)) by [@chaliy](https://github.com/chaliy)
- feat(reporting): complete reporting product surface by [@chaliy](https://github.com/chaliy)
- fix(ui): standardize organization label by [@chaliy](https://github.com/chaliy)
- fix(memory): wire memory store for grpc workers by [@chaliy](https://github.com/chaliy)
- chore(specs): add maintenance completeness checks by [@chaliy](https://github.com/chaliy)
- docs(railway): update release template guidance by [@chaliy](https://github.com/chaliy)
- feat(ui): add provider model counts and edit page by [@chaliy](https://github.com/chaliy)

## [0.8.31] - 2026-05-17

### Highlights

- **Free Communication Protocol (FCP) app channel** - New text-first HTTP inbound channel for App invocation alongside AG-UI and A2A ([#1827](https://github.com/everruns/everruns/pull/1827)).
- **A2A HMAC request signing** - Opt-in HMAC request signing on outbound A2A delegation guards against replay attacks ([#1826](https://github.com/everruns/everruns/pull/1826)).

### What's Changed

- fix(reporting): escape LF-prefixed CSV formulas by [@chaliy](https://github.com/chaliy)
- feat(fcp): Free Communication Protocol app channel ([#1827](https://github.com/everruns/everruns/pull/1827)) by [@chaliy](https://github.com/chaliy)
- fix(ui): default AG-UI channels to generated token ([#1828](https://github.com/everruns/everruns/pull/1828)) by [@chaliy](https://github.com/chaliy)
- feat(a2a): opt-in HMAC request signing for replay protection ([#1826](https://github.com/everruns/everruns/pull/1826)) by [@chaliy](https://github.com/chaliy)
- refactor(config): simplify deployment env vars by [@chaliy](https://github.com/chaliy)
- docs: restructure documentation along Diataxis quadrants ([#1824](https://github.com/everruns/everruns/pull/1824)) by [@chaliy](https://github.com/chaliy)
- fix(core): classify OpenAI insufficient_quota as provider_misconfigured ([#1819](https://github.com/everruns/everruns/pull/1819)) by [@chaliy](https://github.com/chaliy)
- fix(server): block DNS-rebinding SSRF in endpoint auth ([#1820](https://github.com/everruns/everruns/pull/1820)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): enforce readonly ancestor writes ([#1821](https://github.com/everruns/everruns/pull/1821)) by [@chaliy](https://github.com/chaliy)
- fix(reporting): scope projector runs to caller org ([#1822](https://github.com/everruns/everruns/pull/1822)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): align discover output schema with response ([#1823](https://github.com/everruns/everruns/pull/1823)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump node from 25-alpine to 26-alpine in /apps/ui ([#1817](https://github.com/everruns/everruns/pull/1817)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump the npm_and_yarn group across 2 directories with 2 updates ([#1818](https://github.com/everruns/everruns/pull/1818)) by [@dependabot](https://github.com/dependabot)
- fix(reporting): make outbox enqueue best-effort on event/session/llm writes ([#1816](https://github.com/everruns/everruns/pull/1816)) by [@chaliy](https://github.com/chaliy)
- docs(readme): rewrite to reflect current platform scope ([#1808](https://github.com/everruns/everruns/pull/1808)) by [@chaliy](https://github.com/chaliy)

## [0.8.30] - 2026-05-11

### Highlights

- **Shared App endpoint auth framework** - App-published endpoints (AG-UI, A2A, webhook) now share a unified inbound auth framework with OIDC/JWT, OAuth2 introspection, HTTP Basic, and mTLS ([#1810](https://github.com/everruns/everruns/pull/1810)).
- **App channel management redesign** - Apps now ship redesigned channel management with publish guards and aligned schedule channel tools.
- **Async reporting foundation** - New async reporting subsystem with capability usage analytics, saved reports, and exports.
- **Source-backed volume sync** - Source-backed volumes can now sync their files and expose source sync controls so agents can reason about and refresh provenance.
- **A2A capability** - Outbound A2A agent card preview in the UI plus per-channel rate limiting at parity with AG-UI ([#1800](https://github.com/everruns/everruns/pull/1800)).
- **Session feature flags in CLI** - CLI now supports per-session feature flag overrides ([#1802](https://github.com/everruns/everruns/pull/1802)).

### What's Changed

- feat(deploy): add Railway template ingress scaffold by [@chaliy](https://github.com/chaliy)
- feat(apps): add endpoint auth for app channels by [@chaliy](https://github.com/chaliy)
- fix(migrations): restore sequential numbering by [@chaliy](https://github.com/chaliy)
- feat(apps): redesign app channel management by [@chaliy](https://github.com/chaliy)
- feat(reporting): add saved reports and exports by [@chaliy](https://github.com/chaliy)
- feat(reporting): add capability usage analytics by [@chaliy](https://github.com/chaliy)
- docs(specs): add shared App endpoint auth framework ([#1810](https://github.com/everruns/everruns/pull/1810)) by [@chaliy](https://github.com/chaliy)
- feat(cli): support session feature flags ([#1802](https://github.com/everruns/everruns/pull/1802)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fetch A2A agent card preview by [@chaliy](https://github.com/chaliy)
- feat(ui): preview A2A agent cards by [@chaliy](https://github.com/chaliy)
- feat(settings): add organization settings groups by [@chaliy](https://github.com/chaliy)
- chore(deps): bump everruns-sdk to 0.1.9 by [@chaliy](https://github.com/chaliy)
- feat(a2a): per-channel rate limit (parity with AG-UI) ([#1800](https://github.com/everruns/everruns/pull/1800)) by [@chaliy](https://github.com/chaliy)
- fix(memory): enforce default-store invariant in storage update ([#1775](https://github.com/everruns/everruns/pull/1775)) by [@chaliy](https://github.com/chaliy)
- chore(apps): drop unused bare app_id from invocation message metadata ([#1797](https://github.com/everruns/everruns/pull/1797)) by [@chaliy](https://github.com/chaliy)
- fix(apps): align schedule channel tools by [@chaliy](https://github.com/chaliy)
- fix(core): harden email recipient validation ([#1780](https://github.com/everruns/everruns/pull/1780)) by [@chaliy](https://github.com/chaliy)
- fix(volumes): block internal git source URLs ([#1781](https://github.com/everruns/everruns/pull/1781)) by [@chaliy](https://github.com/chaliy)
- feat(api): add utoipa annotations to A2A admin endpoints ([#1795](https://github.com/everruns/everruns/pull/1795)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): harden oauth refresh rotation ([#1788](https://github.com/everruns/everruns/pull/1788)) by [@chaliy](https://github.com/chaliy)
- fix(cursor): verify Valkey tarball integrity ([#1778](https://github.com/everruns/everruns/pull/1778)) by [@chaliy](https://github.com/chaliy)
- docs(specs): document app-channel session ownership invariant ([#1794](https://github.com/everruns/everruns/pull/1794)) by [@chaliy](https://github.com/chaliy)
- fix(apps): require channel before publish by [@chaliy](https://github.com/chaliy)
- feat(volumes): add source sync controls by [@chaliy](https://github.com/chaliy)
- feat(models): allow editing model provider by [@chaliy](https://github.com/chaliy)
- fix(apps): remove experimental gate by [@chaliy](https://github.com/chaliy)
- feat(reporting): build async reporting foundation by [@chaliy](https://github.com/chaliy)
- fix(ui): handle legacy capability configs by [@chaliy](https://github.com/chaliy)
- feat(ui): add full-page skill view by [@chaliy](https://github.com/chaliy)
- fix(memory): resolve configured/default persistent memory store (EVE-459) ([#1783](https://github.com/everruns/everruns/pull/1783)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): hide HTTP metadata from discovery by [@chaliy](https://github.com/chaliy)
- fix(apps): accept five-field schedule cron by [@chaliy](https://github.com/chaliy)
- feat(volumes): sync source-backed volume files by [@chaliy](https://github.com/chaliy)
- chore(skills): refresh agent-browser skill (observability dashboard) ([#1777](https://github.com/everruns/everruns/pull/1777)) by [@chaliy](https://github.com/chaliy)

## [0.8.29] - 2026-05-08

### Highlights

- **Realtime voice sessions** - New voice capability allows sessions to interact via realtime voice with hardened authorization on tool execution ([#1767](https://github.com/everruns/everruns/pull/1767)).
- **Skills discovery** - Skills page now ships search, view dialog, and usage counts so operators can find and adopt skills faster ([#1751](https://github.com/everruns/everruns/pull/1751)).
- **Memory store management** - Memory stores can now be renamed and toggled as default through a new PATCH endpoint ([#1696](https://github.com/everruns/everruns/pull/1696)), with a redesigned memory stores page UX ([#1750](https://github.com/everruns/everruns/pull/1750)).
- **Source-backed volumes** - Workspace volumes now carry source-backed metadata so agents can reason about volume provenance.
- **Cursor host in everruns-dev plugin** - The everruns-dev plugin now supports Cursor as a host alongside Claude Code and Codex ([#1754](https://github.com/everruns/everruns/pull/1754)).
- **Auth hardening** - Generic login errors hide credential validity ([#1763](https://github.com/everruns/everruns/pull/1763)), registration enforces password minimums ([#1762](https://github.com/everruns/everruns/pull/1762)), refresh token rotation is now atomic ([#1761](https://github.com/everruns/everruns/pull/1761)), and Google OAuth requires `email_verified` plus an allowed domain ([#1759](https://github.com/everruns/everruns/pull/1759)).

### What's Changed

- feat(volumes): add source-backed volume metadata by [@chaliy](https://github.com/chaliy)
- fix(cursor): make cloud Dockerfile build with Valkey on amd64 ([#1773](https://github.com/everruns/everruns/pull/1773)) by [@chaliy](https://github.com/chaliy)
- fix(skills): hide deleted skills from usage and content ([#1770](https://github.com/everruns/everruns/pull/1770)) by [@chaliy](https://github.com/chaliy)
- feat(core): add system email sender by [@chaliy](https://github.com/chaliy)
- test(apps): pin shared session reuse by [@chaliy](https://github.com/chaliy)
- fix(voice): disable unauthorized realtime tool execution ([#1767](https://github.com/everruns/everruns/pull/1767)) by [@chaliy](https://github.com/chaliy)
- feat(memory): add PATCH endpoint to rename memory store and toggle default ([#1696](https://github.com/everruns/everruns/pull/1696)) by [@chaliy](https://github.com/chaliy)
- fix(apps): audit app-channel invocations by [@chaliy](https://github.com/chaliy)
- fix(server): resolve org for volume IDs ([#1734](https://github.com/everruns/everruns/pull/1734)) by [@chaliy](https://github.com/chaliy)
- fix(cloud-agent): verify doppler tarball checksum in Dockerfile ([#1765](https://github.com/everruns/everruns/pull/1765)) by [@chaliy](https://github.com/chaliy)
- fix(llm-models): enforce enabled flag in model resolution paths ([#1730](https://github.com/everruns/everruns/pull/1730)) by [@chaliy](https://github.com/chaliy)
- fix(agents): enforce high-risk checks on version activation ([#1749](https://github.com/everruns/everruns/pull/1749)) by [@chaliy](https://github.com/chaliy)
- fix(server): avoid double-applying message query window ([#1746](https://github.com/everruns/everruns/pull/1746)) by [@chaliy](https://github.com/chaliy)
- fix(server): bound harness usage count fan-out ([#1740](https://github.com/everruns/everruns/pull/1740)) by [@chaliy](https://github.com/chaliy)
- fix(ui): hide raw chat failure diagnostics ([#1743](https://github.com/everruns/everruns/pull/1743)) by [@chaliy](https://github.com/chaliy)
- fix(a2a): align stream taskId with session task identity ([#1745](https://github.com/everruns/everruns/pull/1745)) by [@chaliy](https://github.com/chaliy)
- fix(server): bound in-memory AG-UI limiter cache per app ([#1742](https://github.com/everruns/everruns/pull/1742)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): block scoped preview commands from read-only query ([#1741](https://github.com/everruns/everruns/pull/1741)) by [@chaliy](https://github.com/chaliy)
- fix(plugin): shorten everruns dev default prompt ([#1766](https://github.com/everruns/everruns/pull/1766)) by [@chaliy](https://github.com/chaliy)
- fix(server): map domain anyhow::bail! cases to typed 4xx errors ([#1764](https://github.com/everruns/everruns/pull/1764)) by [@chaliy](https://github.com/chaliy)
- fix(auth): use generic login errors for credential failures ([#1763](https://github.com/everruns/everruns/pull/1763)) by [@chaliy](https://github.com/chaliy)
- fix(auth): enforce password minimum on registration API ([#1762](https://github.com/everruns/everruns/pull/1762)) by [@chaliy](https://github.com/chaliy)
- fix(server): authorize AG-UI before parsing JSON body ([#1755](https://github.com/everruns/everruns/pull/1755)) by [@chaliy](https://github.com/chaliy)
- fix(core): strip human_intent from client-side tool calls ([#1756](https://github.com/everruns/everruns/pull/1756)) by [@chaliy](https://github.com/chaliy)
- feat(organizations): add resolve_org command for entity-id lookup ([#1758](https://github.com/everruns/everruns/pull/1758)) by [@chaliy](https://github.com/chaliy)
- fix(events): bound q filter for list_events ([#1737](https://github.com/everruns/everruns/pull/1737)) by [@chaliy](https://github.com/chaliy)
- fix(core): avoid utf8 panic in prompt canary truncation ([#1744](https://github.com/everruns/everruns/pull/1744)) by [@chaliy](https://github.com/chaliy)
- fix(auth): make refresh token rotation atomic ([#1761](https://github.com/everruns/everruns/pull/1761)) by [@chaliy](https://github.com/chaliy)
- fix(ui): restrict MCP card resources to ui://everruns ([#1736](https://github.com/everruns/everruns/pull/1736)) by [@chaliy](https://github.com/chaliy)
- fix(durable): direct workflow lookup for detail and SSE ([#1760](https://github.com/everruns/everruns/pull/1760)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add search, view dialog, and usage counts ([#1751](https://github.com/everruns/everruns/pull/1751)) by [@chaliy](https://github.com/chaliy)
- fix(auth): enforce Google OAuth email_verified and allowed-domain ([#1759](https://github.com/everruns/everruns/pull/1759)) by [@chaliy](https://github.com/chaliy)
- chore(skills): upgrade agent-browser skill to discovery stub ([#1757](https://github.com/everruns/everruns/pull/1757)) by [@chaliy](https://github.com/chaliy)
- feat(plugin): add cursor host to everruns-dev plugin ([#1754](https://github.com/everruns/everruns/pull/1754)) by [@chaliy](https://github.com/chaliy)
- chore(cloud-agent): add repo environment setup ([#1753](https://github.com/everruns/everruns/pull/1753)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): count card sessions by internal agent id ([#1733](https://github.com/everruns/everruns/pull/1733)) by [@chaliy](https://github.com/chaliy)
- fix(ui): handle null workspace volume configs ([#1731](https://github.com/everruns/everruns/pull/1731)) by [@chaliy](https://github.com/chaliy)
- fix(memory): redesign memory stores page UX ([#1750](https://github.com/everruns/everruns/pull/1750)) by [@chaliy](https://github.com/chaliy)
- chore(plugin): sync everruns-dev claude manifest with codex ([#1752](https://github.com/everruns/everruns/pull/1752)) by [@chaliy](https://github.com/chaliy)
- fix(memory): honor capability store config in memory tools ([#1729](https://github.com/everruns/everruns/pull/1729)) by [@chaliy](https://github.com/chaliy)
- fix(ui): block unsafe memory image media types ([#1728](https://github.com/everruns/everruns/pull/1728)) by [@chaliy](https://github.com/chaliy)
- fix(a2a): block stream on shared-session channels ([#1727](https://github.com/everruns/everruns/pull/1727)) by [@chaliy](https://github.com/chaliy)
- fix(cursor): remove global api key fallback ([#1726](https://github.com/everruns/everruns/pull/1726)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): block oauth token use in explicit scoped servers ([#1714](https://github.com/everruns/everruns/pull/1714)) by [@chaliy](https://github.com/chaliy)
- feat(voice): add realtime voice sessions by [@chaliy](https://github.com/chaliy)
- chore(maintenance): add deepsec scan workflow by [@chaliy](https://github.com/chaliy)

## [0.8.28] - 2026-05-07

### Highlights

- **Agent versioning** - Agent definitions can now be versioned, letting operators iterate on agent configurations safely while keeping a stable production version.
- **A2A inbound and outbound parity** - Inbound A2A channels now support `message/stream` SSE ([#1716](https://github.com/everruns/everruns/pull/1716)) and `tasks/get`/`tasks/cancel` with derived state ([#1720](https://github.com/everruns/everruns/pull/1720)); outbound A2A delegation is now governed by the network access policy ([#1711](https://github.com/everruns/everruns/pull/1711)).
- **Declarative capabilities** - Capabilities can now be defined declaratively, with a runtime gate that blocks high-risk declarative dependencies ([#1710](https://github.com/everruns/everruns/pull/1710)).
- **Cursor cloud agents integration** - New first-class integration for running Everruns agents from Cursor's cloud agents surface.
- **MCP app resource rendering in chat** - Chat now renders `ui://everruns/...` MCP app resources inline so agents can return rich UI cards.
- **Machine payment authority** - New payment authority surface lets agents and machine clients act on behalf of an authenticated principal for paid actions ([#1703](https://github.com/everruns/everruns/pull/1703)).
- **Apps channels UI redesign** - The app channels management UI has been redesigned for clarity and faster channel configuration.

### What's Changed

- feat(capabilities): add declarative capabilities by [@chaliy](https://github.com/chaliy)
- feat(payments): add machine payment authority ([#1703](https://github.com/everruns/everruns/pull/1703)) by [@chaliy](https://github.com/chaliy)
- docs(reporting): evaluate StarRocks and DuckDB backends ([#1707](https://github.com/everruns/everruns/pull/1707)) by [@chaliy](https://github.com/chaliy)
- feat(events): list_events debug filters + events_summary tool ([#1706](https://github.com/everruns/everruns/pull/1706)) by [@chaliy](https://github.com/chaliy)
- docs(getting-started): add Claude Code section to Use in AI Tools ([#1702](https://github.com/everruns/everruns/pull/1702)) by [@chaliy](https://github.com/chaliy)
- fix(apps): preserve app owner across channel ingress + wire tests into CI ([#1701](https://github.com/everruns/everruns/pull/1701)) by [@chaliy](https://github.com/chaliy)
- docs(a2a): cross-link inbound channel and outbound capability specs ([#1708](https://github.com/everruns/everruns/pull/1708)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): remove stateless org switch tool by [@chaliy](https://github.com/chaliy)
- fix(durable): reduce turn startup latency by [@chaliy](https://github.com/chaliy)
- fix(payments): disable redirects in payment authority HTTP client ([#1709](https://github.com/everruns/everruns/pull/1709)) by [@chaliy](https://github.com/chaliy)
- feat(a2a): implement message/stream SSE on inbound channel ([#1716](https://github.com/everruns/everruns/pull/1716)) by [@chaliy](https://github.com/chaliy)
- feat(chat): render MCP app resources by [@chaliy](https://github.com/chaliy)
- fix(capabilities): gate declarative high-risk dependencies ([#1710](https://github.com/everruns/everruns/pull/1710)) by [@chaliy](https://github.com/chaliy)
- feat(ui): search organisations in command palette by [@chaliy](https://github.com/chaliy)
- feat(cursor): add cloud agents integration by [@chaliy](https://github.com/chaliy)
- feat(a2a): add tasks/get and tasks/cancel with derived state ([#1720](https://github.com/everruns/everruns/pull/1720)) by [@chaliy](https://github.com/chaliy)
- chore(specs): design realtime voice sessions by [@chaliy](https://github.com/chaliy)
- feat(apps): redesign app channels UI by [@chaliy](https://github.com/chaliy)
- fix(core): enforce network policy for A2A delegation ([#1711](https://github.com/everruns/everruns/pull/1711)) by [@chaliy](https://github.com/chaliy)
- fix(ui): bump next to 16.2.6 for security patches ([#1732](https://github.com/everruns/everruns/pull/1732)) by [@chaliy](https://github.com/chaliy)
- test(worker): scaffold turmoil-based transport reliability tests ([#1724](https://github.com/everruns/everruns/pull/1724)) by [@chaliy](https://github.com/chaliy)
- fix(server): keep AG-UI open through tool commentary by [@chaliy](https://github.com/chaliy)
- feat(agent-versions): add flagged agent versioning by [@chaliy](https://github.com/chaliy)
- fix(ui): skip auth proxy redirects when AUTH_MODE is none by [@chaliy](https://github.com/chaliy)
- fix(server): harden public AG-UI image uploads ([#1712](https://github.com/everruns/everruns/pull/1712)) by [@chaliy](https://github.com/chaliy)

## [0.8.27] - 2026-05-07

### Highlights

- **A2A (Agent2Agent) channel** - Apps can now expose agents over the A2A JSON-RPC protocol with API key auth, and agents can delegate outbound to other A2A agents ([#1695](https://github.com/everruns/everruns/pull/1695)).
- **Knowledge Bases foundation** - New first-class Knowledge Bases entity with curated entries and an agent-facing `search_knowledge` capability ([#1688](https://github.com/everruns/everruns/pull/1688)).
- **Org memory stores** - Persistent cross-session memory now ships with org-scoped stores (API + UI), a memory detail drawer, and tag filtering on the Memory page ([#1687](https://github.com/everruns/everruns/pull/1687), [#1689](https://github.com/everruns/everruns/pull/1689), [#1693](https://github.com/everruns/everruns/pull/1693)).
- **MCP Apps entity-card standard** - New `agent_get_card` tool plus the MCP Apps entity-card spec gives clients a standard way to render rich agent cards from MCP responses ([#1691](https://github.com/everruns/everruns/pull/1691)).
- **Session context usage report** - Sessions now expose a structured context usage report so operators and agents can see token budget consumption ([#1694](https://github.com/everruns/everruns/pull/1694)).

### What's Changed

- chore(maintenance): refresh docs and dependency locks by [@chaliy](https://github.com/chaliy)
- feat(ag-ui): support public image uploads by [@chaliy](https://github.com/chaliy)
- feat(memory): add org memory stores API and UI ([#1687](https://github.com/everruns/everruns/pull/1687)) by [@chaliy](https://github.com/chaliy)
- feat(knowledge): add Knowledge Bases foundation (EVE-423) ([#1688](https://github.com/everruns/everruns/pull/1688)) by [@chaliy](https://github.com/chaliy)
- chore(specs): add reporting design by [@chaliy](https://github.com/chaliy)
- feat(memory): add memory detail drawer on Memory page ([#1689](https://github.com/everruns/everruns/pull/1689)) by [@chaliy](https://github.com/chaliy)
- feat(memory): add tag filter UI on Memory page ([#1693](https://github.com/everruns/everruns/pull/1693)) by [@chaliy](https://github.com/chaliy)
- test(volumes): add UI test cases for workspace volumes ([#1692](https://github.com/everruns/everruns/pull/1692)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add context usage report ([#1694](https://github.com/everruns/everruns/pull/1694)) by [@chaliy](https://github.com/chaliy)
- feat(apps): add A2A (Agent2Agent) channel with API key auth ([#1695](https://github.com/everruns/everruns/pull/1695)) by [@chaliy](https://github.com/chaliy)
- feat(core): add outbound A2A delegation by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP Apps entity-card standard with agent_get_card tool ([#1691](https://github.com/everruns/everruns/pull/1691)) by [@chaliy](https://github.com/chaliy)
- test(memory): add UI test cases for memory stores ([#1697](https://github.com/everruns/everruns/pull/1697)) by [@chaliy](https://github.com/chaliy)
- chore(ship): surface follow-ups explicitly in ship workflow ([#1698](https://github.com/everruns/everruns/pull/1698)) by [@chaliy](https://github.com/chaliy)

## [0.8.26] - 2026-05-06

### Highlights

- **Workspace volumes** - Sessions can now declare reusable workspace volumes through new CRUD APIs, a dedicated UI for managing them, and a capability mount picker that lets agents bind volumes via `workspace_volumes` ([#1682](https://github.com/everruns/everruns/pull/1682)).
- **AG-UI public surface hardening** - Public AG-UI now supports per-channel tokens for scoped access, redacts public tool activity to keep internal details private, and emits structured ingress timing so operators can attribute pre-LLM latency ([#1672](https://github.com/everruns/everruns/pull/1672)).
- **Capabilities catalog rebuild** - The capabilities page now ships search, categories, usage stats, and direct documentation links so operators can discover and adopt capabilities faster ([#1668](https://github.com/everruns/everruns/pull/1668)).
- **MCP robustness** - MCP dispatch now emits structured errors with broader bool coercion ([#1670](https://github.com/everruns/everruns/pull/1670)) and walks `allOf`/`$ref` to coerce JSON aggregate flags for flatten commands ([#1678](https://github.com/everruns/everruns/pull/1678)), tightening the contract for generated bashkit builtins.
- **UI polish** - Per-page document titles ([#1665](https://github.com/everruns/everruns/pull/1665)), models grouped by enabled state with recency sorting and release dates ([#1664](https://github.com/everruns/everruns/pull/1664)), markdown link icons, and improved SEO titles & descriptions across the app.

### What's Changed

- chore(release): tighten highlights guidance for maintenance releases ([#1661](https://github.com/everruns/everruns/pull/1661)) by [@chaliy](https://github.com/chaliy)
- feat(ag-ui): add channel token support by [@chaliy](https://github.com/chaliy)
- feat(ui): group models by enabled, sort by recency, surface release date ([#1664](https://github.com/everruns/everruns/pull/1664)) by [@chaliy](https://github.com/chaliy)
- feat(ui): set per-page document titles ([#1665](https://github.com/everruns/everruns/pull/1665)) by [@chaliy](https://github.com/chaliy)
- fix(billing): omit event_id FK for ephemeral llm.generation events ([#1666](https://github.com/everruns/everruns/pull/1666)) by [@chaliy](https://github.com/chaliy)
- docs(plugin): clarify everruns-dev skill by [@chaliy](https://github.com/chaliy)
- fix(llm-models): accept lenient bool coercion for MCP update_model ([#1667](https://github.com/everruns/everruns/pull/1667)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): structured dispatch errors and broader bool coercion sweep ([#1670](https://github.com/everruns/everruns/pull/1670)) by [@chaliy](https://github.com/chaliy)
- feat(ag-ui): redact public tool activity by [@chaliy](https://github.com/chaliy)
- feat(capabilities): rebuild page with search, categories, stats, docs links ([#1668](https://github.com/everruns/everruns/pull/1668)) by [@chaliy](https://github.com/chaliy)
- chore(llm): log read failures with org/resource context ([#1671](https://github.com/everruns/everruns/pull/1671)) by [@chaliy](https://github.com/chaliy)
- feat(ag-ui): emit structured ingress timing for pre-LLM latency budget ([#1672](https://github.com/everruns/everruns/pull/1672)) by [@chaliy](https://github.com/chaliy)
- docs(getting-started): add Codex plugin setup guide by [@chaliy](https://github.com/chaliy)
- docs(seo): improve page titles and descriptions by [@chaliy](https://github.com/chaliy)
- refactor(llm-models): derive `healthy` from provider, drop status ([#1674](https://github.com/everruns/everruns/pull/1674)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add markdown link icons by [@chaliy](https://github.com/chaliy)
- feat(volumes): add workspace volume CRUD APIs by [@chaliy](https://github.com/chaliy)
- refactor(api): introduce Dispatcher chokepoint and instrument Command::run ([#1679](https://github.com/everruns/everruns/pull/1679)) by [@chaliy](https://github.com/chaliy)
- feat(volumes): add workspace volumes UI by [@chaliy](https://github.com/chaliy)
- fix(mcp): walk allOf/$ref to coerce JSON aggregate flags for flatten cmds ([#1678](https://github.com/everruns/everruns/pull/1678)) by [@chaliy](https://github.com/chaliy)
- fix(deps): clear dependency security alerts by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add volume mount picker for workspace_volumes ([#1682](https://github.com/everruns/everruns/pull/1682)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump oxfmt + document cargo-outdated gap ([#1683](https://github.com/everruns/everruns/pull/1683)) by [@chaliy](https://github.com/chaliy)

## [0.8.25] - 2026-05-04

### Highlights

- **App lifecycle deepens** - Apps gain configurable AG-UI thread expiration ([#1646](https://github.com/everruns/everruns/pull/1646)), per-app rate limits on the public AG-UI endpoint ([#1647](https://github.com/everruns/everruns/pull/1647)), app/channel-scoped budgets with periodic resets ([#1649](https://github.com/everruns/everruns/pull/1649)), session provenance tracking, and a guard that blocks deletion of app-backed entities so live channels stay consistent.
- **Streaming output guardrails + prompt canary capability** - New core capability lets agents validate streaming output and surface prompt canaries for safer model interactions ([#1651](https://github.com/everruns/everruns/pull/1651)).
- **Agent and harness usage stats** - UI now surfaces per-agent and per-harness usage counts so operators can see where activity actually concentrates.
- **AG-UI public-endpoint hardening** - Public AG-UI errors are sanitized and now follow the defined public-endpoint contract ([#1645](https://github.com/everruns/everruns/pull/1645)), privileged message roles are rejected in AG-UI requests ([#1653](https://github.com/everruns/everruns/pull/1653)), and unsafe redirect URI schemes are rejected during MCP OAuth registration ([#1652](https://github.com/everruns/everruns/pull/1652)).
- **bashkit bumped to v0.4.1 from crates.io** - Picks up the latest sandbox capabilities and hardening, and switches the dependency to crates.io ([#1650](https://github.com/everruns/everruns/pull/1650), [#1656](https://github.com/everruns/everruns/pull/1656)).

### What's Changed

- fix(ui): handle missing initial_files in agent and harness preview ([#1634](https://github.com/everruns/everruns/pull/1634)) by [@chaliy](https://github.com/chaliy)
- fix(ui): improve app 404 resource states ([#1638](https://github.com/everruns/everruns/pull/1638)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add shared page layout shell and apply to models page ([#1635](https://github.com/everruns/everruns/pull/1635)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify organisations settings by [@chaliy](https://github.com/chaliy)
- fix(ui): smooth cross-org entity links by [@chaliy](https://github.com/chaliy)
- fix(api): add UI links to command outputs by [@chaliy](https://github.com/chaliy)
- fix(worker): notify initial durable tasks via active backend by [@chaliy](https://github.com/chaliy)
- test(mcp): add adversarial org-scope coverage by [@chaliy](https://github.com/chaliy)
- fix(deletion): block deleting app-backed entities by [@chaliy](https://github.com/chaliy)
- feat(ui): show agent and harness usage counts by [@chaliy](https://github.com/chaliy)
- fix(core): cap infinity context prompt window by [@chaliy](https://github.com/chaliy)
- feat(apps): add configurable AG-UI thread expiration ([#1646](https://github.com/everruns/everruns/pull/1646)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to v0.4.0 ([#1650](https://github.com/everruns/everruns/pull/1650)) by [@chaliy](https://github.com/chaliy)
- fix(server): reject unsafe redirect URI schemes in MCP OAuth registration ([#1652](https://github.com/everruns/everruns/pull/1652)) by [@chaliy](https://github.com/chaliy)
- fix(server): reject privileged message roles in AG-UI request body ([#1653](https://github.com/everruns/everruns/pull/1653)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): accept array and object flags in generated bashkit builtins ([#1654](https://github.com/everruns/everruns/pull/1654)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show display names in selects, never internal values ([#1648](https://github.com/everruns/everruns/pull/1648)) by [@chaliy](https://github.com/chaliy)
- fix(server): sanitize AG-UI public errors + define public-endpoint contract ([#1645](https://github.com/everruns/everruns/pull/1645)) by [@chaliy](https://github.com/chaliy)
- feat(ag-ui): configurable per-app rate limit on public endpoint ([#1647](https://github.com/everruns/everruns/pull/1647)) by [@chaliy](https://github.com/chaliy)
- feat(budgets): app/channel scoped budgets with periodic resets ([#1649](https://github.com/everruns/everruns/pull/1649)) by [@chaliy](https://github.com/chaliy)
- feat(stats): add agent and harness usage stats by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to v0.4.1 from crates.io ([#1656](https://github.com/everruns/everruns/pull/1656)) by [@chaliy](https://github.com/chaliy)
- feat(apps): track app session provenance by [@chaliy](https://github.com/chaliy)
- fix(ui): show chat error alerts by [@chaliy](https://github.com/chaliy)
- feat(core): streaming output guardrails + prompt canary capability ([#1651](https://github.com/everruns/everruns/pull/1651)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): rebuild dev showcases with real components ([#1657](https://github.com/everruns/everruns/pull/1657)) by [@chaliy](https://github.com/chaliy)
- chore(ship): validate migration ordering after rebase and pre-merge ([#1659](https://github.com/everruns/everruns/pull/1659)) by [@chaliy](https://github.com/chaliy)

## [0.8.24] - 2026-05-03

### Highlights

- **Harness examples gallery** - New gallery surfaces built-in harness examples so users can browse and adopt them more easily ([#1629](https://github.com/everruns/everruns/pull/1629)).
- **MCP discover exposes output schemas** - `discover` now includes command output schemas, giving MCP clients richer typing information ([#1627](https://github.com/everruns/everruns/pull/1627)).
- **MCP OAuth metadata reliability** - Path-specific OAuth protected-resource metadata URL fixes resource discovery for clients that probe per-endpoint metadata ([#1623](https://github.com/everruns/everruns/pull/1623)), and the Everruns Dev plugin declares its `oauth_resource` so Codex MCP login satisfies RFC 8707 ([#1624](https://github.com/everruns/everruns/pull/1624)).
- **bashkit bumped to v0.3.0** - Picks up the latest sandbox capabilities and hardening from upstream bashkit ([#1630](https://github.com/everruns/everruns/pull/1630), [#1625](https://github.com/everruns/everruns/pull/1625)).
- **Everruns Dev plugin docs** - Plugin README and metadata aligned with the Claude Code spec for clearer surfacing in marketplaces ([#1628](https://github.com/everruns/everruns/pull/1628)).

### What's Changed

- fix(plugin): add Everruns Dev MCP OAuth resource ([#1624](https://github.com/everruns/everruns/pull/1624)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): use path-specific OAuth protected-resource metadata URL ([#1623](https://github.com/everruns/everruns/pull/1623)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to v0.2.1 ([#1625](https://github.com/everruns/everruns/pull/1625)) by [@chaliy](https://github.com/chaliy)
- chore(plugins): enforce maintenance parity checks by [@chaliy](https://github.com/chaliy)
- fix(mcp): expose command output schemas in discover ([#1627](https://github.com/everruns/everruns/pull/1627)) by [@chaliy](https://github.com/chaliy)
- docs(plugin): align everruns-dev with Claude Code spec and surface in README ([#1628](https://github.com/everruns/everruns/pull/1628)) by [@chaliy](https://github.com/chaliy)
- feat(harnesses): add harness examples gallery ([#1629](https://github.com/everruns/everruns/pull/1629)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to v0.3.0 ([#1630](https://github.com/everruns/everruns/pull/1630)) by [@chaliy](https://github.com/chaliy)

## [0.8.23] - 2026-04-30

### Highlights

- **Default model bumped to `gpt-5.5`** - The out-of-the-box default model is now `gpt-5.5`, replacing the previous default. Existing agents and sessions with explicit `default_model_id` are unaffected.
- **Embeddable auth & search MCP integration** - New `ServerAppBuilder` hook lets embedders wrap the API key CRUD router ([#1617](https://github.com/everruns/everruns/pull/1617)), and the parallel-search MCP integration adds a first-class search backend that capabilities can consume.
- **Human-intent tool hooks** - Core gains explicit hooks for human-intent tool flows, giving capabilities a structured place to participate in user-driven tool execution.
- **Per-request HTTP access logs** - Server now emits one `INFO` log per HTTP request with `request_id`, status, and latency ([#1614](https://github.com/everruns/everruns/pull/1614)), so operators get a uniform request audit trail without enabling debug-level logs.
- **Auth hardening** - External OAuth misconfiguration now fails fast at startup, the signup harness-seed safety net is restored via `platform_definition`, and `/btw` LLM errors are classified instead of returning `500` ([#1613](https://github.com/everruns/everruns/pull/1613)).

### What's Changed

- fix(commands): classify /btw LLM errors instead of returning 500 ([#1613](https://github.com/everruns/everruns/pull/1613)) by [@chaliy](https://github.com/chaliy)
- feat(server): emit one INFO log per HTTP request with request_id, status, latency ([#1614](https://github.com/everruns/everruns/pull/1614)) by [@chaliy](https://github.com/chaliy)
- feat(core): add human intent tool hooks by [@chaliy](https://github.com/chaliy)
- fix(discovery): add homepage link headers by [@chaliy](https://github.com/chaliy)
- feat(auth): add ServerAppBuilder hook to wrap API key CRUD router ([#1617](https://github.com/everruns/everruns/pull/1617)) by [@chaliy](https://github.com/chaliy)
- feat(parallel): add search mcp integration by [@chaliy](https://github.com/chaliy)
- chore(agents): make agents dirs source of truth ([#1618](https://github.com/everruns/everruns/pull/1618)) by [@chaliy](https://github.com/chaliy)
- chore(docs): keep proposals out of public docs by [@chaliy](https://github.com/chaliy)
- fix(auth): restore signup harness-seed safety net via platform_definition by [@chaliy](https://github.com/chaliy)
- fix(auth): fail fast on external OAuth config by [@chaliy](https://github.com/chaliy)
- chore(plugins): update everruns dev metadata by [@chaliy](https://github.com/chaliy)
- feat(models): default to gpt-5.5 by [@chaliy](https://github.com/chaliy)

## [0.8.22] - 2026-04-27

### Highlights

- **Model Router & workspace volumes scaffolding** - Foundation slices land for the upcoming Model Router ([EVE-397](https://linear.app/everruns/issue/EVE-397)) and `workspace_volumes` capability ([EVE-396](https://linear.app/everruns/issue/EVE-396)): durable specs, typed IDs, migrations `025_volumes.sql` / `026_model_routers.sql`, core types with structural validation, and capability registration. CRUD APIs, runtime resolution, and UI ship as follow-up vertical slices; existing concrete `default_model_id` behavior is preserved.
- **GitHub Enterprise Daytona clone-auth** - `daytona_git_clone` and `daytona_git_credentials` honor an operator-configured trusted-host allowlist (`EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS`), so Enterprise customers can authenticate against `github.acme.com` / `git.internal.corp` while public-SaaS behavior and lookalike-host rejection are unchanged. See `integrations/daytona/SPEC.md` and `knowledge/security/threat-model.md` (TM-DAYTONA-008).
- **Schema-driven capability settings** - Capability config UIs are now rendered from capability-provided JSON Schema metadata; per-capability settings editors disappear from shared UI code and server-side config validation is centralized.
- **Security hardening sweep** - Public `/metrics` endpoint disabled by default ([#1596](https://github.com/everruns/everruns/pull/1596)), Valkey auth rate-limiter fails closed on backend errors ([#1597](https://github.com/everruns/everruns/pull/1597)), admin-seed reuse requires `admin` role + `email_verified` ([#1590](https://github.com/everruns/everruns/pull/1590)), `bashkit` bumped to v0.1.21 catching up on three security/hardening releases ([#1606](https://github.com/everruns/everruns/pull/1606)), and `virtual_bash` / `web_fetch` admin-only tier is now an explicit product-tier decision (specs + decision comments).
- **Tools-field deprecation window** - `CreateSessionRequest.tools`, `CreateAgentRequest.tools`, and `UpdateAgentRequest.tools` now soft-drop legacy non-`client_side` entries with a structured `tracing::warn!` instead of returning `400`. Operators can flip `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS=1` in dev/staging to surface remaining offenders before the strict cutover. See `knowledge/execution/client-side-tools.md` (Tools-Field Deprecation Window).

### What's Changed

- feat(model-router): add Model Router scaffolding ([#1610](https://github.com/everruns/everruns/pull/1610)) by [@chaliy](https://github.com/chaliy)
- feat(volumes): add workspace_volumes capability scaffolding ([#1609](https://github.com/everruns/everruns/pull/1609)) by [@chaliy](https://github.com/chaliy)
- docs(capabilities): document admin-only tier for virtual_bash and web_fetch ([#1608](https://github.com/everruns/everruns/pull/1608)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit to v0.1.21 ([#1606](https://github.com/everruns/everruns/pull/1606)) by [@chaliy](https://github.com/chaliy)
- fix(server): stage client_side tools rejection with deprecation window ([#1602](https://github.com/everruns/everruns/pull/1602)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): support GitHub Enterprise clone-auth allowlist ([#1601](https://github.com/everruns/everruns/pull/1601)) by [@chaliy](https://github.com/chaliy)
- fix(cli): allow opt-in hidden directories in initial_files ([#1600](https://github.com/everruns/everruns/pull/1600)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prune unsequenced usage IDs on SSE trim ([#1598](https://github.com/everruns/everruns/pull/1598)) by [@chaliy](https://github.com/chaliy)
- fix(server): fail closed on valkey auth rate-limit errors ([#1597](https://github.com/everruns/everruns/pull/1597)) by [@chaliy](https://github.com/chaliy)
- fix(server): disable public /metrics by default ([#1596](https://github.com/everruns/everruns/pull/1596)) by [@chaliy](https://github.com/chaliy)
- fix(server): block admin seed promotion for untrusted email ([#1590](https://github.com/everruns/everruns/pull/1590)) by [@chaliy](https://github.com/chaliy)
- feat(settings): split organisations page by [@chaliy](https://github.com/chaliy)
- feat(capabilities): render config settings from schemas by [@chaliy](https://github.com/chaliy)
- chore(migrations): enforce migration immutability by [@chaliy](https://github.com/chaliy)

## [0.8.21] - 2026-04-25

### Highlights

- **Hotfix: bound migration 024 input to fit `to_tsvector` 1 MiB cap** - The v0.8.20 search-vector rebuild migration crashed dev startup with `string is too long for tsvector (3108640 bytes, max 1048575 bytes)` because individual event rows can carry tool results, accumulated streaming text, or message-content arrays well beyond Postgres' 1 MiB tsvector input limit. The canonical-text expression is now wrapped in `LEFT(..., 250000)` so any single row's contribution stays under tsvector's hard cap. Search relevance is dominated by early tokens, so the truncation has negligible effect on the index's usefulness.
- **GPT-5.5 / GPT-5.5 Pro** - New OpenAI model entries are wired into the LLM driver registry ([#1593](https://github.com/everruns/everruns/pull/1593)).

### What's Changed

- fix(migrations): bound migration 024 input to 250 000 chars so to_tsvector stays within Postgres' 1 MiB cap by [@chaliy](https://github.com/chaliy)
- feat(llm): add GPT-5.5 and GPT-5.5 Pro ([#1593](https://github.com/everruns/everruns/pull/1593)) by [@chaliy](https://github.com/chaliy)

## [0.8.20] - 2026-04-25

### Highlights

- **Hotfix: restore migration 006 checksum** - Migration `006_v0.8.5.sql` was edited in-place by [#1566](https://github.com/everruns/everruns/pull/1566) to broaden `events.search_vector` coverage to nested message text. Modifying an applied migration breaks startup against existing databases because sqlx tracks per-migration checksums in `_sqlx_migrations`. v0.8.20 reverts migration 006 to its original v0.8.18 form and re-delivers the search-vector update as a new additive migration `024_event_search_vector_canonical_fields.sql`. Restores deploys against dev/prod databases.
- **UI: SVG previews behind sandboxed iframe** - SVG file previews are restored under a sandboxed iframe so user-supplied vectors can't break out into the host page ([#1587](https://github.com/everruns/everruns/pull/1587)).

### What's Changed

- fix(migrations): restore migration 006 checksum; re-deliver search_vector via additive 024 by [@chaliy](https://github.com/chaliy)
- fix(ui): restore SVG previews behind sandboxed iframe ([#1587](https://github.com/everruns/everruns/pull/1587)) by [@chaliy](https://github.com/chaliy)
- chore(plugin): rename everruns dev plugin by [@chaliy](https://github.com/chaliy)
- feat(ui): move models to building blocks by [@chaliy](https://github.com/chaliy)

## [0.8.19] - 2026-04-24

### Highlights

- **Security hardening sweep** - Broad audit pass across auth, sessions, MCP, durable execution, CLI, and UI: API-key revalidation, CSRF-confirmed MCP OAuth, bounded buffers, UTF-8 panic guards, path/symlink traversal blocks, ACL and policy enforcement via `Command::run`, and redacted tool timelines. See `knowledge/security/threat-model.md`.
- **OpenAI image generation** - `gpt-image-2` now streams generation progress and the end-to-end pipeline is reliable for everyday use.
- **Raw session file downloads** - New API endpoint exposes raw session file downloads for external consumers ([#1443](https://github.com/everruns/everruns/pull/1443)).
- **MCP `WWW-Authenticate` on 401 (RFC 9728)** - `/mcp` endpoints now emit `WWW-Authenticate` on 401 so compliant clients can discover the OAuth resource server automatically ([#1441](https://github.com/everruns/everruns/pull/1441)).
- **Multitenancy auto-select** - Direct links to a resource now auto-select the owning organization, removing a sharp UX edge when users belong to multiple orgs ([#1450](https://github.com/everruns/everruns/pull/1450)).

### Security

- Document the `activate_skill` ``!`command` `` trust gate. The gate remains forced off for every source because `SessionFile::is_readonly` is user-controllable via the session-files API and `InitialFile`. `preprocess_command_injections` is kept wired up (with bounded fan-out: 32 placeholders per activation, 4 concurrent shells) so a follow-up can flip it on once a platform-controlled provenance signal is added to `SessionFile`. See `knowledge/project/skills-registry.md` "Activation Substitution Pipeline" and threat-model entry TM-TOOL-020 (EVE-388).
- Restore SVG file preview behind a sandboxed `<iframe sandbox="" srcDoc=...>` with a strict CSP meta tag (`default-src 'none'; style-src 'unsafe-inline'; img-src data:`). PR #1513 had blocked SVG previews entirely to close an XSS surface; the iframe + CSP gate restores the feature while keeping `<script>`, `on*`, `javascript:`, and `<foreignObject>` payloads inert. See threat-model entry TM-WEB-009 (EVE-389).

### What's Changed

- fix(skills): keep !`command` gated off; document re-enable prerequisites ([#1581](https://github.com/everruns/everruns/pull/1581)) by [@chaliy](https://github.com/chaliy)
- build(deps): bump postcss from 8.5.8 to 8.5.10 in /apps/docs in the npm_and_yarn group across 1 directory ([#1584](https://github.com/everruns/everruns/pull/1584)) by [@dependabot](https://github.com/apps/dependabot)
- fix(platform-chat): enforce platform tool permissions ([#1582](https://github.com/everruns/everruns/pull/1582)) by [@chaliy](https://github.com/chaliy)
- chore(docs): fix broken spec cross-refs and index stale references ([#1585](https://github.com/everruns/everruns/pull/1585)) by [@chaliy](https://github.com/chaliy)
- test(server): cover owner-scoped connection resolution ([#1574](https://github.com/everruns/everruns/pull/1574)) by [@chaliy](https://github.com/chaliy)
- build(deps): bump rustls-webpki from 0.103.12 to 0.103.13 in /examples/weekend-concierge-host in the cargo group across 1 directory ([#1583](https://github.com/everruns/everruns/pull/1583)) by [@dependabot](https://github.com/apps/dependabot)
- test(cli): regression tests for connections set argv key leak ([#1580](https://github.com/everruns/everruns/pull/1580)) by [@chaliy](https://github.com/chaliy)
- feat(openai): stream image generation progress by [@chaliy](https://github.com/chaliy)
- test(capabilities): lock in high-risk gating for bash/fetch ([#1579](https://github.com/everruns/everruns/pull/1579)) by [@chaliy](https://github.com/chaliy)
- test(slack): cross-org recovery app lookup rejection ([#1578](https://github.com/everruns/everruns/pull/1578)) by [@chaliy](https://github.com/chaliy)
- test(mcp): policy enforcement on resources/read list commands ([#1577](https://github.com/everruns/everruns/pull/1577)) by [@chaliy](https://github.com/chaliy)
- test(auth): regression tests for API key revalidation ([#1576](https://github.com/everruns/everruns/pull/1576)) by [@chaliy](https://github.com/chaliy)
- fix(storage): preserve forward in-memory event catch-up window ([#1563](https://github.com/everruns/everruns/pull/1563)) by [@chaliy](https://github.com/chaliy)
- fix(server): handle existing admin email during seed ([#1561](https://github.com/everruns/everruns/pull/1561)) by [@chaliy](https://github.com/chaliy)
- fix(apps): re-encrypt legacy channel configs on app update ([#1472](https://github.com/everruns/everruns/pull/1472)) by [@chaliy](https://github.com/chaliy)
- fix(skills): preserve user-invocable flag on skill update ([#1569](https://github.com/everruns/everruns/pull/1569)) by [@chaliy](https://github.com/chaliy)
- fix(ui): sync org cookie on automatic org fallback ([#1558](https://github.com/everruns/everruns/pull/1558)) by [@chaliy](https://github.com/chaliy)
- fix(server): preserve activity type in NATS notifications ([#1555](https://github.com/everruns/everruns/pull/1555)) by [@chaliy](https://github.com/chaliy)
- fix(server): block deleted status in manage update endpoints ([#1494](https://github.com/everruns/everruns/pull/1494)) by [@chaliy](https://github.com/chaliy)
- fix(worker): block pending takeover with claimed tasks ([#1557](https://github.com/everruns/everruns/pull/1557)) by [@chaliy](https://github.com/chaliy)
- fix(anthropic): avoid utf8 panic in model id normalization ([#1521](https://github.com/everruns/everruns/pull/1521)) by [@chaliy](https://github.com/chaliy)
- fix(ui): block route-manifest path traversal ([#1544](https://github.com/everruns/everruns/pull/1544)) by [@chaliy](https://github.com/chaliy)
- fix(ui): guard invalid schedule timezone rendering ([#1515](https://github.com/everruns/everruns/pull/1515)) by [@chaliy](https://github.com/chaliy)
- fix(ui): block svg file previews ([#1513](https://github.com/everruns/everruns/pull/1513)) by [@chaliy](https://github.com/chaliy)
- fix(otel): preserve parent-child span nesting ([#1573](https://github.com/everruns/everruns/pull/1573)) by [@chaliy](https://github.com/chaliy)
- fix(server): refresh stale MCP caches in batch load ([#1572](https://github.com/everruns/everruns/pull/1572)) by [@chaliy](https://github.com/chaliy)
- fix(migrations): backfill owner for legacy org memberships ([#1571](https://github.com/everruns/everruns/pull/1571)) by [@chaliy](https://github.com/chaliy)
- fix(durable): bind correct params when resetting workflow pending ([#1570](https://github.com/everruns/everruns/pull/1570)) by [@chaliy](https://github.com/chaliy)
- fix(ui): avoid global chat init when flag disabled ([#1568](https://github.com/everruns/everruns/pull/1568)) by [@chaliy](https://github.com/chaliy)
- fix(dev): bind Caddy admin API to localhost ([#1567](https://github.com/everruns/everruns/pull/1567)) by [@chaliy](https://github.com/chaliy)
- fix(storage): search nested message content in event filters ([#1566](https://github.com/everruns/everruns/pull/1566)) by [@chaliy](https://github.com/chaliy)
- fix(server): bound turn-prefix pagination query ([#1565](https://github.com/everruns/everruns/pull/1565)) by [@chaliy](https://github.com/chaliy)
- fix(durable): handle snapshot sequence zero correctly ([#1564](https://github.com/everruns/everruns/pull/1564)) by [@chaliy](https://github.com/chaliy)
- fix(ui): keep live usage updating after SSE trim cap ([#1562](https://github.com/everruns/everruns/pull/1562)) by [@chaliy](https://github.com/chaliy)
- fix(ui): avoid render-phase state updates in command and file views ([#1560](https://github.com/everruns/everruns/pull/1560)) by [@chaliy](https://github.com/chaliy)
- fix(skills): preserve disable-model-invocation on update ([#1559](https://github.com/everruns/everruns/pull/1559)) by [@chaliy](https://github.com/chaliy)
- fix(ci): avoid exposing PAT in homebrew tap clone ([#1556](https://github.com/everruns/everruns/pull/1556)) by [@chaliy](https://github.com/chaliy)
- fix(setup): avoid top-level local in Linux NATS install ([#1554](https://github.com/everruns/everruns/pull/1554)) by [@chaliy](https://github.com/chaliy)
- fix(core): restore bash narration for commands arg ([#1553](https://github.com/everruns/everruns/pull/1553)) by [@chaliy](https://github.com/chaliy)
- fix(server): handle NULL display_name in harness search ([#1552](https://github.com/everruns/everruns/pull/1552)) by [@chaliy](https://github.com/chaliy)
- fix(ci): restore Linux formula URL on AMD x86_64 ([#1551](https://github.com/everruns/everruns/pull/1551)) by [@chaliy](https://github.com/chaliy)
- fix(evals): prevent orphaned eval runs without a target ([#1550](https://github.com/everruns/everruns/pull/1550)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): handle root virtual mount lookups ([#1549](https://github.com/everruns/everruns/pull/1549)) by [@chaliy](https://github.com/chaliy)
- fix(container-sandbox): bound Docker API response buffering ([#1548](https://github.com/everruns/everruns/pull/1548)) by [@chaliy](https://github.com/chaliy)
- fix(core): avoid double-applying capability configs ([#1547](https://github.com/everruns/everruns/pull/1547)) by [@chaliy](https://github.com/chaliy)
- fix(worker): preserve steering signals across act/failure ([#1546](https://github.com/everruns/everruns/pull/1546)) by [@chaliy](https://github.com/chaliy)
- fix(docs): block unsafe notebook markdown URLs ([#1545](https://github.com/everruns/everruns/pull/1545)) by [@chaliy](https://github.com/chaliy)
- fix(evals): prevent NameError in SWE-bench score write-back ([#1543](https://github.com/everruns/everruns/pull/1543)) by [@chaliy](https://github.com/chaliy)
- fix(core): avoid UTF-8 panic when truncating AGENTS.md ([#1542](https://github.com/everruns/everruns/pull/1542)) by [@chaliy](https://github.com/chaliy)
- fix(core): escape AGENTS.md XML content in prompts ([#1541](https://github.com/everruns/everruns/pull/1541)) by [@chaliy](https://github.com/chaliy)
- fix(server): clamp turn context message limits ([#1540](https://github.com/everruns/everruns/pull/1540)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): scope unpin to caller org ([#1539](https://github.com/everruns/everruns/pull/1539)) by [@chaliy](https://github.com/chaliy)
- fix(ci): run rust-gated jobs when CLI E2E script changes ([#1538](https://github.com/everruns/everruns/pull/1538)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): reject traversal segments in default clone path ([#1537](https://github.com/everruns/everruns/pull/1537)) by [@chaliy](https://github.com/chaliy)
- fix(auth): sanitize register return_to redirect ([#1536](https://github.com/everruns/everruns/pull/1536)) by [@chaliy](https://github.com/chaliy)
- fix(server): bound Slack users.info cache growth ([#1535](https://github.com/everruns/everruns/pull/1535)) by [@chaliy](https://github.com/chaliy)
- fix(slack): restrict manifest endpoint exposure ([#1534](https://github.com/everruns/everruns/pull/1534)) by [@chaliy](https://github.com/chaliy)
- fix(server): keep auth rate limiter fail-open on valkey errors ([#1533](https://github.com/everruns/everruns/pull/1533)) by [@chaliy](https://github.com/chaliy)
- fix(auth): always revalidate API keys against DB ([#1532](https://github.com/everruns/everruns/pull/1532)) by [@chaliy](https://github.com/chaliy)
- fix(server): share LLM resolver with worker service ([#1531](https://github.com/everruns/everruns/pull/1531)) by [@chaliy](https://github.com/chaliy)
- fix(encryption): validate key_id in DEK cache lookup ([#1530](https://github.com/everruns/everruns/pull/1530)) by [@chaliy](https://github.com/chaliy)
- fix(server): preserve readonly guard in write_file race recovery ([#1529](https://github.com/everruns/everruns/pull/1529)) by [@chaliy](https://github.com/chaliy)
- fix(slack): skip bot replies in thread context injection ([#1528](https://github.com/everruns/everruns/pull/1528)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): persist child session metadata ([#1527](https://github.com/everruns/everruns/pull/1527)) by [@chaliy](https://github.com/chaliy)
- fix(server): block virtual mount create-path writes ([#1526](https://github.com/everruns/everruns/pull/1526)) by [@chaliy](https://github.com/chaliy)
- fix(server): restrict client tool definitions to client_side ([#1525](https://github.com/everruns/everruns/pull/1525)) by [@chaliy](https://github.com/chaliy)
- fix(core): ignore untrusted metadata locale override ([#1524](https://github.com/everruns/everruns/pull/1524)) by [@chaliy](https://github.com/chaliy)
- fix(durable): enforce post-load event cap during replay ([#1523](https://github.com/everruns/everruns/pull/1523)) by [@chaliy](https://github.com/chaliy)
- fix(cli): block symlink traversal in file sync paths ([#1520](https://github.com/everruns/everruns/pull/1520)) by [@chaliy](https://github.com/chaliy)
- fix(api): avoid UTF-8 boundary panic in OAuth error truncation ([#1519](https://github.com/everruns/everruns/pull/1519)) by [@chaliy](https://github.com/chaliy)
- fix(cli): avoid API key in connections set args ([#1518](https://github.com/everruns/everruns/pull/1518)) by [@chaliy](https://github.com/chaliy)
- fix(cli): block hidden initial_files directories ([#1517](https://github.com/everruns/everruns/pull/1517)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): enforce policies for resources/read metadata ([#1516](https://github.com/everruns/everruns/pull/1516)) by [@chaliy](https://github.com/chaliy)
- fix(api-keys): restore per-user API key quota ([#1514](https://github.com/everruns/everruns/pull/1514)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): gate high-risk harness capabilities for members ([#1485](https://github.com/everruns/everruns/pull/1485)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): harden git credential file permissions ([#1509](https://github.com/everruns/everruns/pull/1509)) by [@chaliy](https://github.com/chaliy)
- fix(core): enforce web_fetch file-download runtime guard ([#1507](https://github.com/everruns/everruns/pull/1507)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): ignore disabled servers in prefix resolver ([#1504](https://github.com/everruns/everruns/pull/1504)) by [@chaliy](https://github.com/chaliy)
- fix(server): enforce Slack team/channel webhook scope ([#1501](https://github.com/everruns/everruns/pull/1501)) by [@chaliy](https://github.com/chaliy)
- fix(slack): scope recovery app lookup to session org ([#1502](https://github.com/everruns/everruns/pull/1502)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): restore high-risk levels for bash/fetch ([#1500](https://github.com/everruns/everruns/pull/1500)) by [@chaliy](https://github.com/chaliy)
- fix(server): cancel SSE pg_notify listener on disconnect ([#1510](https://github.com/everruns/everruns/pull/1510)) by [@chaliy](https://github.com/chaliy)
- fix(server): block built-in harness mutations in platform store ([#1508](https://github.com/everruns/everruns/pull/1508)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): validate persisted CDP reconnect endpoint ([#1505](https://github.com/everruns/everruns/pull/1505)) by [@chaliy](https://github.com/chaliy)
- fix(slack): validate attachment image URLs before LLM forwarding ([#1503](https://github.com/everruns/everruns/pull/1503)) by [@chaliy](https://github.com/chaliy)
- fix(server): scope global chat singleton by owner principal ([#1499](https://github.com/everruns/everruns/pull/1499)) by [@chaliy](https://github.com/chaliy)
- fix(core): stop logging full OpenResponses request bodies ([#1522](https://github.com/everruns/everruns/pull/1522)) by [@chaliy](https://github.com/chaliy)
- fix(gemini): move API key from URL to auth header ([#1511](https://github.com/everruns/everruns/pull/1511)) by [@chaliy](https://github.com/chaliy)
- fix(notifications): prevent SSE replay loop on cursor poll ([#1506](https://github.com/everruns/everruns/pull/1506)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): verify doppler tarball checksum ([#1512](https://github.com/everruns/everruns/pull/1512)) by [@chaliy](https://github.com/chaliy)
- fix(ci): validate docker release dispatch tag ref ([#1498](https://github.com/everruns/everruns/pull/1498)) by [@chaliy](https://github.com/chaliy)
- fix(skills): bound prompt-time skill discovery scan ([#1497](https://github.com/everruns/everruns/pull/1497)) by [@chaliy](https://github.com/chaliy)
- refactor(session_resources): absorb SessionResourceService into queries ([#1496](https://github.com/everruns/everruns/pull/1496)) by [@chaliy](https://github.com/chaliy)
- refactor(services): drop deprecated shims for moved modules ([#1495](https://github.com/everruns/everruns/pull/1495)) by [@chaliy](https://github.com/chaliy)
- fix(auth): disable OAuth in external auth mode ([#1492](https://github.com/everruns/everruns/pull/1492)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): restrict git clone auth to github host ([#1491](https://github.com/everruns/everruns/pull/1491)) by [@chaliy](https://github.com/chaliy)
- fix(server): block cross-user GitHub installation linking ([#1490](https://github.com/everruns/everruns/pull/1490)) by [@chaliy](https://github.com/chaliy)
- fix(server): remove privileged tools from platform chat ([#1489](https://github.com/everruns/everruns/pull/1489)) by [@chaliy](https://github.com/chaliy)
- fix(server): scope session connection lookups to owner user ([#1487](https://github.com/everruns/everruns/pull/1487)) by [@chaliy](https://github.com/chaliy)
- fix(authz): restore missing view policy checks ([#1486](https://github.com/everruns/everruns/pull/1486)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): reject reserved internal tags on update ([#1484](https://github.com/everruns/everruns/pull/1484)) by [@chaliy](https://github.com/chaliy)
- fix(server): reject JWT auth for deleted users ([#1470](https://github.com/everruns/everruns/pull/1470)) by [@chaliy](https://github.com/chaliy)
- fix(core): cap persisted tool output to prevent storage DoS ([#1469](https://github.com/everruns/everruns/pull/1469)) by [@chaliy](https://github.com/chaliy)
- fix(session_sandbox): strip session-level Daytona base URL overrides ([#1458](https://github.com/everruns/everruns/pull/1458)) by [@chaliy](https://github.com/chaliy)
- fix(core): bound background output buffering memory ([#1456](https://github.com/everruns/everruns/pull/1456)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redact timeline tool outputs for secret_store ([#1493](https://github.com/everruns/everruns/pull/1493)) by [@chaliy](https://github.com/chaliy)
- refactor(services): move single-owner services into their owning domains ([#1483](https://github.com/everruns/everruns/pull/1483)) by [@chaliy](https://github.com/chaliy)
- refactor(authz): retire #[policy] macro now that Command::run enforces ([#1482](https://github.com/everruns/everruns/pull/1482)) by [@chaliy](https://github.com/chaliy)
- fix(authz): enforce command policy on every caller via Command::run ([#1451](https://github.com/everruns/everruns/pull/1451)) by [@chaliy](https://github.com/chaliy)
- fix(harnesses): require HARNESS_VIEW for preview access ([#1478](https://github.com/everruns/everruns/pull/1478)) by [@chaliy](https://github.com/chaliy)
- fix(deno): bound sandbox stream buffer growth ([#1476](https://github.com/everruns/everruns/pull/1476)) by [@chaliy](https://github.com/chaliy)
- fix(subagents): enforce blueprint capability authorization ([#1475](https://github.com/everruns/everruns/pull/1475)) by [@chaliy](https://github.com/chaliy)
- fix(org): require admin for harness defaults updates ([#1481](https://github.com/everruns/everruns/pull/1481)) by [@chaliy](https://github.com/chaliy)
- fix(worker): exclude dependency blockers from LLM breaker ([#1480](https://github.com/everruns/everruns/pull/1480)) by [@chaliy](https://github.com/chaliy)
- fix(compaction): avoid UTF-8 panic in summarization truncation ([#1479](https://github.com/everruns/everruns/pull/1479)) by [@chaliy](https://github.com/chaliy)
- fix(server): enforce one-time CLI exchange code use ([#1477](https://github.com/everruns/everruns/pull/1477)) by [@chaliy](https://github.com/chaliy)
- fix(ui): harden mermaid rendering with strict security mode ([#1474](https://github.com/everruns/everruns/pull/1474)) by [@chaliy](https://github.com/chaliy)
- fix(server): harden API rate-limit client IP extraction ([#1471](https://github.com/everruns/everruns/pull/1471)) by [@chaliy](https://github.com/chaliy)
- fix(server): prevent max_iterations cast overflow ([#1467](https://github.com/everruns/everruns/pull/1467)) by [@chaliy](https://github.com/chaliy)
- fix(core): reapply read_file hard cap after outline append ([#1464](https://github.com/everruns/everruns/pull/1464)) by [@chaliy](https://github.com/chaliy)
- fix(sprites): validate sprite names when loading state ([#1473](https://github.com/everruns/everruns/pull/1473)) by [@chaliy](https://github.com/chaliy)
- fix(budgets): enforce manage policy for budget mutations ([#1468](https://github.com/everruns/everruns/pull/1468)) by [@chaliy](https://github.com/chaliy)
- fix(core): protect internal secret_store namespaces ([#1466](https://github.com/everruns/everruns/pull/1466)) by [@chaliy](https://github.com/chaliy)
- fix(core): enforce ACLs for blueprint sessions ([#1465](https://github.com/everruns/everruns/pull/1465)) by [@chaliy](https://github.com/chaliy)
- fix(evals): require session manage permission to run evals ([#1463](https://github.com/everruns/everruns/pull/1463)) by [@chaliy](https://github.com/chaliy)
- fix(auth): stop reseeding harnesses in public signup flows ([#1462](https://github.com/everruns/everruns/pull/1462)) by [@chaliy](https://github.com/chaliy)
- fix(container-sandbox): avoid sandbox name collisions ([#1461](https://github.com/everruns/everruns/pull/1461)) by [@chaliy](https://github.com/chaliy)
- fix(ci): remove unpinned cargo-binstall install script ([#1460](https://github.com/everruns/everruns/pull/1460)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): preserve session blueprint for act tools ([#1457](https://github.com/everruns/everruns/pull/1457)) by [@chaliy](https://github.com/chaliy)
- fix(core): gate Braintrust tool labels on args mode ([#1454](https://github.com/everruns/everruns/pull/1454)) by [@chaliy](https://github.com/chaliy)
- fix(commands): enforce session view policy on command execution ([#1453](https://github.com/everruns/everruns/pull/1453)) by [@chaliy](https://github.com/chaliy)
- fix(server): reject scoped MCP names with reserved delimiter ([#1455](https://github.com/everruns/everruns/pull/1455)) by [@chaliy](https://github.com/chaliy)
- fix(apps): prevent shared invocation session hijacking ([#1452](https://github.com/everruns/everruns/pull/1452)) by [@chaliy](https://github.com/chaliy)
- feat(multitenancy): auto-select owning org from direct resource links ([#1450](https://github.com/everruns/everruns/pull/1450)) by [@chaliy](https://github.com/chaliy)
- fix(ui): stop trusting forwarded headers for SSR API origin ([#1459](https://github.com/everruns/everruns/pull/1459)) by [@chaliy](https://github.com/chaliy)
- feat(openai): make gpt-image-2 image generation reliable by [@chaliy](https://github.com/chaliy)
- fix(auth): require CSRF-confirmed MCP OAuth authorization ([#1447](https://github.com/everruns/everruns/pull/1447)) by [@chaliy](https://github.com/chaliy)
- fix(mcp): enforce api key organization scope for overrides ([#1446](https://github.com/everruns/everruns/pull/1446)) by [@chaliy](https://github.com/chaliy)
- fix(skills): disable command placeholder execution in activation ([#1449](https://github.com/everruns/everruns/pull/1449)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): suppress interact content when secrets used ([#1448](https://github.com/everruns/everruns/pull/1448)) by [@chaliy](https://github.com/chaliy)
- fix(openai): prevent session base URL credential leak ([#1445](https://github.com/everruns/everruns/pull/1445)) by [@chaliy](https://github.com/chaliy)
- feat(api): add raw session file downloads ([#1443](https://github.com/everruns/everruns/pull/1443)) by [@chaliy](https://github.com/chaliy)
- refactor(server): move domain services into domains by [@chaliy](https://github.com/chaliy)
- feat(mcp): emit WWW-Authenticate on /mcp 401 responses (RFC 9728) ([#1441](https://github.com/everruns/everruns/pull/1441)) by [@chaliy](https://github.com/chaliy)

## [0.8.18] - 2026-04-22

### Highlights

- **App invocation channels** - Apps can now be invoked via schedules and webhooks, expanding deployment options beyond agents and chat channels ([#1431](https://github.com/everruns/everruns/pull/1431))
- **Draft apps** - Apps can be created and saved without committing to an agent or channel, enabling staged authoring ([#1415](https://github.com/everruns/everruns/pull/1415))
- **Budget usage journal & ledger** - New extensible budgeting backbone tracks LLM and tool usage with a journal and ledger for fine-grained cost accounting ([#1434](https://github.com/everruns/everruns/pull/1434))
- **MCP read-only query tool & metadata** - Standardized MCP server tool metadata and added a read-only query tool for safe data access ([#1435](https://github.com/everruns/everruns/pull/1435), [#1418](https://github.com/everruns/everruns/pull/1418))
- **Claude Code MCP plugin** - New Everruns MCP plugin and marketplace entry for direct integration with Claude Code ([#1406](https://github.com/everruns/everruns/pull/1406))
- **SWE-bench Lite eval harness** - Added evaluation harness for benchmarking against SWE-bench Lite ([#1394](https://github.com/everruns/everruns/pull/1394))
- **Prompt cache request metadata** - LLM driver now propagates prompt cache metadata for improved observability and tuning ([#1398](https://github.com/everruns/everruns/pull/1398))

### What's Changed

- feat(daytona): standardize exec output contract ([#1390](https://github.com/everruns/everruns/pull/1390)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): suffix sandbox names on create ([#1396](https://github.com/everruns/everruns/pull/1396)) by [@chaliy](https://github.com/chaliy)
- fix(integrations): scope sandbox resources to sessions ([#1397](https://github.com/everruns/everruns/pull/1397)) by [@chaliy](https://github.com/chaliy)
- fix(runtime): unify overlay capability execution path by [@chaliy](https://github.com/chaliy)
- chore(ci): codify integration live-test policy ([#1399](https://github.com/everruns/everruns/pull/1399)) by [@chaliy](https://github.com/chaliy)
- feat(evals): add SWE-bench Lite evaluation harness ([#1394](https://github.com/everruns/everruns/pull/1394)) by [@chaliy](https://github.com/chaliy)
- feat(llm): add prompt cache request metadata ([#1398](https://github.com/everruns/everruns/pull/1398)) by [@chaliy](https://github.com/chaliy)
- fix(ci): stop doc-only pushes from triggering paid live tests ([#1405](https://github.com/everruns/everruns/pull/1405)) by [@chaliy](https://github.com/chaliy)
- feat(embedding): add vendor-neutral error-reporting hooks for wrappers by [@chaliy](https://github.com/chaliy)
- fix(core): redesign Braintrust REST delivery ([#1403](https://github.com/everruns/everruns/pull/1403)) by [@chaliy](https://github.com/chaliy)
- fix(core): unify exec presentation contract ([#1402](https://github.com/everruns/everruns/pull/1402)) by [@chaliy](https://github.com/chaliy)
- test(integrations): add Gemini and Daytona regression coverage ([#1400](https://github.com/everruns/everruns/pull/1400)) by [@chaliy](https://github.com/chaliy)
- test(mcp): add argument and notification contract matrix by [@chaliy](https://github.com/chaliy)
- fix(ci): scope live jobs to real integration changes ([#1407](https://github.com/everruns/everruns/pull/1407)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): attach page target for CDP v2 ([#1410](https://github.com/everruns/everruns/pull/1410)) by [@chaliy](https://github.com/chaliy)
- feat(claude-code-plugin): add Everruns MCP plugin and marketplace ([#1406](https://github.com/everruns/everruns/pull/1406)) by [@chaliy](https://github.com/chaliy)
- test(scripts): formalize shell helper CI coverage ([#1409](https://github.com/everruns/everruns/pull/1409)) by [@chaliy](https://github.com/chaliy)
- fix(deno): resolve sandbox A records explicitly by [@chaliy](https://github.com/chaliy)
- chore(maintenance): require Claude Code plugin freshness review by [@chaliy](https://github.com/chaliy)
- refactor(errors): use structured runtime error metadata ([#1414](https://github.com/everruns/everruns/pull/1414)) by [@chaliy](https://github.com/chaliy)
- feat(apps): allow draft apps without agent or channel ([#1415](https://github.com/everruns/everruns/pull/1415)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): standardize server tool metadata ([#1418](https://github.com/everruns/everruns/pull/1418)) by [@chaliy](https://github.com/chaliy)
- fix(server): enable bashkit jq feature for MCP execute tool ([#1419](https://github.com/everruns/everruns/pull/1419)) by [@chaliy](https://github.com/chaliy)
- refactor(server): move request types from api/ to domains/*/types.rs ([#1416](https://github.com/everruns/everruns/pull/1416)) by [@chaliy](https://github.com/chaliy)
- refactor(server): use bashkit async_tool for MCP catalog callbacks ([#1420](https://github.com/everruns/everruns/pull/1420)) by [@chaliy](https://github.com/chaliy)
- feat(core): add observational bashkit hooks for virtual_bash ([#1421](https://github.com/everruns/everruns/pull/1421)) by [@chaliy](https://github.com/chaliy)
- feat(core): unify reading-tool truncation envelope ([#1422](https://github.com/everruns/everruns/pull/1422)) by [@chaliy](https://github.com/chaliy)
- chore(release): remove migration squash requirement ([#1423](https://github.com/everruns/everruns/pull/1423)) by [@chaliy](https://github.com/chaliy)
- fix(ci): pin deno live tests to ams ([#1417](https://github.com/everruns/everruns/pull/1417)) by [@chaliy](https://github.com/chaliy)
- fix(plugins): share everruns plugin root ([#1424](https://github.com/everruns/everruns/pull/1424)) by [@chaliy](https://github.com/chaliy)
- refactor(server): migrate remaining API domains to commands by [@chaliy](https://github.com/chaliy)
- fix(sprites): align live API and CI token checks ([#1426](https://github.com/everruns/everruns/pull/1426)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump astro from 6.1.3 to 6.1.8 in /apps/docs in the npm_and_yarn group across 1 directory ([#1427](https://github.com/everruns/everruns/pull/1427)) by [@dependabot](https://github.com/dependabot)
- fix(session-sandbox): harden managed sandbox e2e ([#1428](https://github.com/everruns/everruns/pull/1428)) by [@chaliy](https://github.com/chaliy)
- refactor(mcp): remove static catalog and handlers ([#1430](https://github.com/everruns/everruns/pull/1430)) by [@chaliy](https://github.com/chaliy)
- fix(api-keys): use expiry presets and drop key quota ([#1432](https://github.com/everruns/everruns/pull/1432)) by [@chaliy](https://github.com/chaliy)
- feat(server): add budget usage journal and ledger ([#1434](https://github.com/everruns/everruns/pull/1434)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add read-only query tool ([#1435](https://github.com/everruns/everruns/pull/1435)) by [@chaliy](https://github.com/chaliy)
- fix(media): preserve image IDs across RPC and API ([#1438](https://github.com/everruns/everruns/pull/1438)) by [@chaliy](https://github.com/chaliy)
- fix(server): gate coding-container behind container sandbox flag by [@chaliy](https://github.com/chaliy)
- test(agents): add image generation artifact case ([#1439](https://github.com/everruns/everruns/pull/1439)) by [@chaliy](https://github.com/chaliy)
- feat(commands): add btw capability ([#1437](https://github.com/everruns/everruns/pull/1437)) by [@chaliy](https://github.com/chaliy)
- feat(apps): add schedule and webhook invocation channels ([#1431](https://github.com/everruns/everruns/pull/1431)) by [@chaliy](https://github.com/chaliy)

## [0.8.17] - 2026-04-19

### Highlights

- **Principal-based ownership** - Durable entities (sessions, schedules, apps) now track ownership via principals, with backfill from existing agent identities, users, and session metadata ([#1367](https://github.com/everruns/everruns/pull/1367))
- **Initial files editor** - Agents and harnesses can be configured with initial files through a new editor and preview flow ([#1380](https://github.com/everruns/everruns/pull/1380), [#1377](https://github.com/everruns/everruns/pull/1377))
- **Idempotent skill activation** - `activate_skill` is now idempotent within a session, eliminating duplicate skill registrations on repeated calls ([#1373](https://github.com/everruns/everruns/pull/1373))
- **UI route manifest** - Wrappers can consume the UI route manifest via a stable export, enabling downstream customization ([#1381](https://github.com/everruns/everruns/pull/1381))
- **Migration-history safety** - Restored `016_eval_case_result_metadata.sql` and `017_eval_artifacts.sql` to preserve compatibility with existing deployments; regression test locks filenames and SQL bodies ([#1382](https://github.com/everruns/everruns/pull/1382))

### What's Changed

- fix(evals): open session links in new tab ([#1365](https://github.com/everruns/everruns/pull/1365)) by [@chaliy](https://github.com/chaliy)
- test(platform_chat): add execute-discover and docs cases ([#1368](https://github.com/everruns/everruns/pull/1368)) by [@chaliy](https://github.com/chaliy)
- fix(ci): repair sprites live-test job-level if ([#1369](https://github.com/everruns/everruns/pull/1369)) by [@chaliy](https://github.com/chaliy)
- fix(ci): add missing container-sandbox live_api_test target ([#1370](https://github.com/everruns/everruns/pull/1370)) by [@chaliy](https://github.com/chaliy)
- test(integrations): fail closed on missing live-test credentials ([#1371](https://github.com/everruns/everruns/pull/1371)) by [@chaliy](https://github.com/chaliy)
- fix(cli): forward EVERRUNS_ORG_ID as X-Org-Id on agent import ([#1372](https://github.com/everruns/everruns/pull/1372)) by [@chaliy](https://github.com/chaliy)
- feat(skills): make activate_skill idempotent within a session ([#1373](https://github.com/everruns/everruns/pull/1373)) by [@chaliy](https://github.com/chaliy)
- feat(authz): make platform-user authz first-class ([#1367](https://github.com/everruns/everruns/pull/1367)) by [@chaliy](https://github.com/chaliy)
- ci: guard release-prep PRs from un-squashed migrations ([#1374](https://github.com/everruns/everruns/pull/1374)) by [@chaliy](https://github.com/chaliy)
- test(ci): add worker PR coverage for durable execution ([#1375](https://github.com/everruns/everruns/pull/1375)) by [@chaliy](https://github.com/chaliy)
- feat(ui): preview initial files for agents and harnesses ([#1377](https://github.com/everruns/everruns/pull/1377)) by [@chaliy](https://github.com/chaliy)
- test(ci): wire UI Playwright smoke into PR CI ([#1376](https://github.com/everruns/everruns/pull/1376)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove Azure OpenAI from org setup ([#1379](https://github.com/everruns/everruns/pull/1379)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add initial files editor ([#1380](https://github.com/everruns/everruns/pull/1380)) by [@chaliy](https://github.com/chaliy)
- feat(ui): expose route manifest for wrappers ([#1381](https://github.com/everruns/everruns/pull/1381)) by [@chaliy](https://github.com/chaliy)
- fix(ui): harden malformed tag rendering ([#1383](https://github.com/everruns/everruns/pull/1383)) by [@chaliy](https://github.com/chaliy)
- fix(server): restore v0.8.16 migration history ([#1382](https://github.com/everruns/everruns/pull/1382)) by [@chaliy](https://github.com/chaliy)
- test(auth): expand org-scoped integration matrix ([#1384](https://github.com/everruns/everruns/pull/1384)) by [@chaliy](https://github.com/chaliy)
- test(ci): wire runtime, CLI, and durable failpoint coverage ([#1385](https://github.com/everruns/everruns/pull/1385)) by [@chaliy](https://github.com/chaliy)
- test(ci): wire 12 server integration tests + fix EVE-195 cli no-org path ([#1378](https://github.com/everruns/everruns/pull/1378)) by [@chaliy](https://github.com/chaliy)
- chore(issue-tracking): tighten issue pickup ownership by [@chaliy](https://github.com/chaliy)
- fix(ui): position notification submenu flyout ([#1392](https://github.com/everruns/everruns/pull/1392)) by [@chaliy](https://github.com/chaliy)
- fix(server): use direct postgres listener urls ([#1389](https://github.com/everruns/everruns/pull/1389)) by [@chaliy](https://github.com/chaliy)
- chore(specs): document postgres listener deployment ([#1393](https://github.com/everruns/everruns/pull/1393)) by [@chaliy](https://github.com/chaliy)
- feat(authz): add principal ownership metadata by [@chaliy](https://github.com/chaliy)

## [0.8.16] - 2026-04-19

### Highlights

- **Data Analyst harness** - New built-in harness tuned for data-analysis workflows ([#1340](https://github.com/everruns/everruns/pull/1340))
- **Azure OpenAI provider** - First-class LLM driver for Azure OpenAI deployments ([#1339](https://github.com/everruns/everruns/pull/1339))
- **A2UI generative UI** - Google A2UI JSON added as a parallel generative-UI capability alongside OpenUI ([#1354](https://github.com/everruns/everruns/pull/1354))
- **Image generation** - New `gpt_image_gen` capability exposes OpenAI image generation to agents ([#1350](https://github.com/everruns/everruns/pull/1350))
- **Self-Budget capability** - Prompt-only `self_budget` teaches agents to reason about user-indicated budgets using cumulative usage from `get_session_info` ([#1342](https://github.com/everruns/everruns/pull/1342))

### What's Changed

- refactor(server): drop hardcoded built-in harness UUIDs ([#1360](https://github.com/everruns/everruns/pull/1360)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): expose command param schemas ([#1363](https://github.com/everruns/everruns/pull/1363)) by [@chaliy](https://github.com/chaliy)
- docs(docs): render notebooks as cookbook pages ([#1352](https://github.com/everruns/everruns/pull/1352)) by [@chaliy](https://github.com/chaliy)
- refactor(worker): unify platform CRUD via execute_command ([#1357](https://github.com/everruns/everruns/pull/1357)) by [@chaliy](https://github.com/chaliy)
- docs(test-cases): add Platform Chat create-and-run agent manual test ([#1362](https://github.com/everruns/everruns/pull/1362)) by [@chaliy](https://github.com/chaliy)
- chore(harness): scrub hardcoded UUID refs from specs and examples ([#1359](https://github.com/everruns/everruns/pull/1359)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add A2UI as parallel generative-UI capability ([#1354](https://github.com/everruns/everruns/pull/1354)) by [@chaliy](https://github.com/chaliy)
- chore(release): document migration squashing in release process ([#1358](https://github.com/everruns/everruns/pull/1358)) by [@chaliy](https://github.com/chaliy)
- refactor(server): delete App/McpServer/Skill services ([#1356](https://github.com/everruns/everruns/pull/1356)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump rust from 1.92-slim-bookworm to 1.95-slim-bookworm in /docker ([#1347](https://github.com/everruns/everruns/pull/1347))
- chore(deps): bump rust from 1.92-slim to 1.95-slim in /crates/worker ([#1346](https://github.com/everruns/everruns/pull/1346))
- chore(deps): bump rust from 1.92-slim to 1.95-slim in /crates/server ([#1345](https://github.com/everruns/everruns/pull/1345))
- chore(deps): bump node from 22-alpine to 25-alpine in /apps/ui ([#1344](https://github.com/everruns/everruns/pull/1344))
- fix(mcp): return 202 for JSON-RPC notifications ([#1355](https://github.com/everruns/everruns/pull/1355)) by [@chaliy](https://github.com/chaliy)
- fix(gemini): strip additionalProperties recursively from tool schemas ([#1353](https://github.com/everruns/everruns/pull/1353)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): reset exec session after timeout ([#1351](https://github.com/everruns/everruns/pull/1351)) by [@chaliy](https://github.com/chaliy)
- feat(media): add gpt_image_gen capability ([#1350](https://github.com/everruns/everruns/pull/1350)) by [@chaliy](https://github.com/chaliy)
- fix(server): repair stale platform chat sessions ([#1349](https://github.com/everruns/everruns/pull/1349)) by [@chaliy](https://github.com/chaliy)
- ci(docker): publish on release only, gate PR builds by paths ([#1343](https://github.com/everruns/everruns/pull/1343)) by [@chaliy](https://github.com/chaliy)
- refactor(server): migrate audit_logs to domains/ pattern ([#1348](https://github.com/everruns/everruns/pull/1348)) by [@chaliy](https://github.com/chaliy)
- fix(docker): harden images against common Dockerfile footguns ([#1331](https://github.com/everruns/everruns/pull/1331)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add prompt-only self_budget capability ([#1342](https://github.com/everruns/everruns/pull/1342)) by [@chaliy](https://github.com/chaliy)
- docs(docs): add notebook-backed tutorial pipeline ([#1341](https://github.com/everruns/everruns/pull/1341)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add Data Analyst built-in harness ([#1340](https://github.com/everruns/everruns/pull/1340)) by [@chaliy](https://github.com/chaliy)
- feat(llm): add Azure OpenAI provider ([#1339](https://github.com/everruns/everruns/pull/1339)) by [@chaliy](https://github.com/chaliy)
- fix(server): accept positional id args on MCP execute commands ([#1338](https://github.com/everruns/everruns/pull/1338)) by [@chaliy](https://github.com/chaliy)

## [0.8.15] - 2026-04-18

### Highlights

- **Embeddable Runtime** - New example demonstrates embedding Everruns as an in-process runtime host ([#1332](https://github.com/everruns/everruns/pull/1332))
- **Scoped MCP Servers** - Remote MCP server configs can now be attached at harness, agent, or session scope and merge with precedence over org-scoped servers ([#1315](https://github.com/everruns/everruns/pull/1315))

### What's Changed

- fix(worker): preserve ActInput.org_id through durable act-task roundtrip ([#1330](https://github.com/everruns/everruns/pull/1330))
- feat(examples): add runtime host example ([#1332](https://github.com/everruns/everruns/pull/1332))
- chore(prompts): adopt Claude 4.7 prompting guidance in capability prompts ([#1329](https://github.com/everruns/everruns/pull/1329))
- feat(capabilities): allow capabilities to contribute skills ([#1328](https://github.com/everruns/everruns/pull/1328))
- feat(mcp): add scoped MCP servers ([#1315](https://github.com/everruns/everruns/pull/1315))
- feat(ui): abstract SSE transport for downstream auth ([#1327](https://github.com/everruns/everruns/pull/1327))

## [0.8.14] - 2026-04-17

### Highlights

- **Scheduled Monitors** - Monitors can now run on a cron schedule, enabling recurring background checks without manual trigger ([#1322](https://github.com/everruns/everruns/pull/1322))
- **Background Tool Execution** - New `spawn_background` meta-tool runs long-lived tools asynchronously with session-resource visibility and completion signalling ([#1308](https://github.com/everruns/everruns/pull/1308))
- **AG-UI App Channel** - Apps can now register against the AG-UI channel alongside Slack ([#1307](https://github.com/everruns/everruns/pull/1307))
- **Claude Opus 4.7** - Added Anthropic profile for Claude Opus 4.7 and promoted it to the seeded favorite Opus model ([#1323](https://github.com/everruns/everruns/pull/1323))
- **Correlation IDs** - Every HTTP request now carries `request_id` and `session_id` through async execution and durable workers for end-to-end log tracing ([#1320](https://github.com/everruns/everruns/pull/1320))

### What's Changed

- refactor(auth): unify login-page resume contract on return_to ([#1325](https://github.com/everruns/everruns/pull/1325))
- feat(runtime): add durable-agnostic host planning ([#1311](https://github.com/everruns/everruns/pull/1311))
- refactor(server): introduce domain modules and delete service layer duplication ([#1318](https://github.com/everruns/everruns/pull/1318))
- chore(deps): bump dompurify from 3.3.3 to 3.4.0 in /apps/ui ([#1312](https://github.com/everruns/everruns/pull/1312))
- feat(observability): add request_id, session_id correlation IDs ([#1320](https://github.com/everruns/everruns/pull/1320))
- feat(session): include usage in get_session_info ([#1324](https://github.com/everruns/everruns/pull/1324))
- feat(models): add Claude Opus 4.7 ([#1323](https://github.com/everruns/everruns/pull/1323))
- fix(worker): emit session.idled when act tasks hit DLQ ([#1321](https://github.com/everruns/everruns/pull/1321))
- feat(monitors): add scheduled monitor support ([#1322](https://github.com/everruns/everruns/pull/1322))
- fix(worker): preserve inner act activity failures ([#1319](https://github.com/everruns/everruns/pull/1319))
- feat(setup): add LLM provider configuration to org setup page ([#1317](https://github.com/everruns/everruns/pull/1317))
- feat(ui): add profile menu extension point ([#1316](https://github.com/everruns/everruns/pull/1316))
- chore(cli): adopt everruns-sdk 0.1.8 ([#1314](https://github.com/everruns/everruns/pull/1314))
- feat(cli): add agent.toml support ([#1313](https://github.com/everruns/everruns/pull/1313))
- feat(apps): add AG-UI app channel ([#1307](https://github.com/everruns/everruns/pull/1307))
- feat(core): add background tool execution ([#1308](https://github.com/everruns/everruns/pull/1308))

## [0.8.13] - 2026-04-15

### Highlights

- **Embedded Runtime** - Everruns can now run embedded in-process with shared turn context and shared host orchestration, which closes the gap between local runtime and worker execution ([#1304](https://github.com/everruns/everruns/pull/1304), [#1306](https://github.com/everruns/everruns/pull/1306), [#1309](https://github.com/everruns/everruns/pull/1309))
- **Managed Session Sandboxes** - Session sandboxes now have an explicit managed lifecycle, which gives runtime-owned sandbox resources the same durability model as other session resources ([#1305](https://github.com/everruns/everruns/pull/1305))
- **UI Flow Tightening** - List pages now hydrate on the server and the shell chrome got smaller cleanup passes around density, warnings, and notifications ([#1299](https://github.com/everruns/everruns/pull/1299), [#1300](https://github.com/everruns/everruns/pull/1300), [#1302](https://github.com/everruns/everruns/pull/1302), [#1303](https://github.com/everruns/everruns/pull/1303))

### What's Changed

- feat(runtime): share host orchestration with worker ([#1309](https://github.com/everruns/everruns/pull/1309))
- feat(runtime): expose shared turn context ([#1306](https://github.com/everruns/everruns/pull/1306))
- feat(session): add managed session sandbox lifecycle ([#1305](https://github.com/everruns/everruns/pull/1305))
- feat(runtime): add embedded in-process runtime ([#1304](https://github.com/everruns/everruns/pull/1304))
- fix(ui): move sidebar notifications into account menu ([#1303](https://github.com/everruns/everruns/pull/1303))
- fix(core): advertise bash help in virtual bash ([#1301](https://github.com/everruns/everruns/pull/1301))
- fix(ui): tighten app density ([#1302](https://github.com/everruns/everruns/pull/1302))
- fix(ui): reuse settings warning notice ([#1300](https://github.com/everruns/everruns/pull/1300))
- chore(specs): drop release migration notes ([#1298](https://github.com/everruns/everruns/pull/1298))
- feat(ui): hydrate list pages on the server ([#1299](https://github.com/everruns/everruns/pull/1299))

## [0.8.12] - 2026-04-14

### Highlights

- **API Keys Are Personal** - API keys are now user-scoped instead of org-scoped, which aligns key auth with session auth and removes org binding drift ([#1293](https://github.com/everruns/everruns/pull/1293))
- **Form Validation Tightening** - Core UI forms now use Zod validation, catching invalid input earlier and making client-side errors more consistent ([#1294](https://github.com/everruns/everruns/pull/1294))
- **Auth + MCP Cleanup** - Auth redirect proxying and MCP endpoint simplification reduce auth friction and remove duplicated endpoint logic ([#1295](https://github.com/everruns/everruns/pull/1295), [#1289](https://github.com/everruns/everruns/pull/1289))

### What's Changed

- chore(deps): bump bashkit to v0.1.18 ([#1296](https://github.com/everruns/everruns/pull/1296))
- fix(ui): add auth redirect proxy ([#1295](https://github.com/everruns/everruns/pull/1295))
- feat(ui): add zod validation to core forms ([#1294](https://github.com/everruns/everruns/pull/1294))
- refactor(auth): make API keys user-scoped instead of org-scoped ([#1293](https://github.com/everruns/everruns/pull/1293))
- refactor(mcp): simplify MCP endpoint, remove duplication ([#1289](https://github.com/everruns/everruns/pull/1289))
- docs: rename Capabilities tab to Built-ins, add harness pages ([#1292](https://github.com/everruns/everruns/pull/1292))
- feat(models): rename installed to enabled for LLM model visibility ([#1291](https://github.com/everruns/everruns/pull/1291))
- fix(ui): remove rounded-* class violations across 55 components ([#1290](https://github.com/everruns/everruns/pull/1290))
- fix(rate-limit): raise default API rate limit from 120 to 1200 req/min ([#1288](https://github.com/everruns/everruns/pull/1288))
- ci: reduce CI waste and improve cache sharing ([#1287](https://github.com/everruns/everruns/pull/1287))

### Migration Notes

**0.8.11 → 0.8.12:** Released `0.8.11` databases migrate forward normally with `013_v0.8.12.sql`. Fresh database required only if you already applied the unreleased `013_rename_installed_to_enabled.sql` and `014_api_keys_user_scoped.sql` migrations from `main`.

## [0.8.11] - 2026-04-13

### Highlights

- **Container Sandbox** — Built-in `coding-container` harness with Docker Engine client, capability/tools scaffold, threat model, docs, and full integration parity ([#1276](https://github.com/everruns/everruns/pull/1276), [#1278](https://github.com/everruns/everruns/pull/1278), [#1279](https://github.com/everruns/everruns/pull/1279))
- **MCP Resources & Multi-Org** — `resources/list` and `resources/read` methods, multi-org OAuth support, direct service call dispatch ([#1259](https://github.com/everruns/everruns/pull/1259), [#1271](https://github.com/everruns/everruns/pull/1271), [#1272](https://github.com/everruns/everruns/pull/1272))
- **Session Resource Registry** — Generic session-scoped resource tracking for sandboxes, subagents, and background work ([#1274](https://github.com/everruns/everruns/pull/1274))
- **Auth Hardening** — Scoped API key cache per org, Secure cookie flag, default org harness init ([#1282](https://github.com/everruns/everruns/pull/1282), [#1284](https://github.com/everruns/everruns/pull/1284), [#1275](https://github.com/everruns/everruns/pull/1275))

### What's Changed

- feat(mcp): replace HTTP/router dispatch with direct service calls ([#1272](https://github.com/everruns/everruns/pull/1272))
- feat(mcp): add multi-org support for OAuth MCP clients ([#1271](https://github.com/everruns/everruns/pull/1271))
- feat(mcp): implement resources/list and resources/read methods ([#1259](https://github.com/everruns/everruns/pull/1259))
- feat(container-sandbox): add Docker Engine REST API client ([#1276](https://github.com/everruns/everruns/pull/1276))
- feat(container-sandbox): add capability, tools, and crate scaffold ([#1278](https://github.com/everruns/everruns/pull/1278))
- feat(container-sandbox): harness, threat model, docs, and integration parity ([#1279](https://github.com/everruns/everruns/pull/1279))
- feat(capabilities): mount platform docs in chat via virtual filesystem ([#1273](https://github.com/everruns/everruns/pull/1273))
- feat(api): session resource registry ([#1274](https://github.com/everruns/everruns/pull/1274))
- feat(flags): enable notifications feature flag in dev ([#1283](https://github.com/everruns/everruns/pull/1283))
- fix(auth): ensure default org gets harnesses via init_org ([#1275](https://github.com/everruns/everruns/pull/1275))
- fix(auth): add missing Secure flag to switch-org cookie ([#1282](https://github.com/everruns/everruns/pull/1282))
- fix(auth): scope API key query cache per org ([#1284](https://github.com/everruns/everruns/pull/1284))
- revert(infra): remove sandbox VPS infra (belongs in SaaS repo) ([#1281](https://github.com/everruns/everruns/pull/1281))
- chore(specs): simplify specs - remove duplication and trim code-mirroring content ([#1285](https://github.com/everruns/everruns/pull/1285))

## [0.8.10] - 2026-04-11

### Highlights

- **MCP Endpoint Improvements** — In-process router dispatch replaces HTTP loopback, fuzzy search in discover, pagination in list tools, local `--help`, cleaner execute output ([#1269](https://github.com/everruns/everruns/pull/1269), [#1255](https://github.com/everruns/everruns/pull/1255), [#1260](https://github.com/everruns/everruns/pull/1260), [#1264](https://github.com/everruns/everruns/pull/1264), [#1265](https://github.com/everruns/everruns/pull/1265))
- **Evals Improvements** — Composable `EvalTarget` replaces fixed harness/agent pair, post messages for benchmark-style scoring, eval run workflow ([#1239](https://github.com/everruns/everruns/pull/1239), [#1248](https://github.com/everruns/everruns/pull/1248))
- **Virtual Readonly Mounts** — Session filesystem gains readonly mount support for injecting host-side content ([#1249](https://github.com/everruns/everruns/pull/1249))

### What's Changed

- feat(capabilities): gate Docker capability behind feature flag ([#1268](https://github.com/everruns/everruns/pull/1268))
- feat(mcp): replace HTTP loopback with in-process router dispatch ([#1269](https://github.com/everruns/everruns/pull/1269))
- feat(fs): add virtual readonly mounts for session filesystem ([#1249](https://github.com/everruns/everruns/pull/1249))
- feat(mcp): add local --help for all built-in commands ([#1265](https://github.com/everruns/everruns/pull/1265))
- feat(mcp): preserve self_url and view_url in execute tool summary output ([#1262](https://github.com/everruns/everruns/pull/1262))
- feat(mcp): simplify execute tool output to plain text ([#1264](https://github.com/everruns/everruns/pull/1264))
- feat(api): add url and view_url fields to all resource API responses ([#1261](https://github.com/everruns/everruns/pull/1261))
- feat(mcp): improve discover tool with fuzzy search, clean output, and --all flag ([#1260](https://github.com/everruns/everruns/pull/1260))
- feat(mcp): improve execute tool description for LLM consumers ([#1263](https://github.com/everruns/everruns/pull/1263))
- feat(mcp): add pagination and summary to all list built-in tools ([#1255](https://github.com/everruns/everruns/pull/1255))
- feat(evals): add post messages and implement eval run workflow ([#1248](https://github.com/everruns/everruns/pull/1248))
- feat(evals): replace harness_id/agent_id with EvalTarget ([#1239](https://github.com/everruns/everruns/pull/1239))
- feat(server): restructure routing and fix MCP OAuth end-to-end ([#1250](https://github.com/everruns/everruns/pull/1250))
- feat(harness): add coding-daytona built-in harness ([#1241](https://github.com/everruns/everruns/pull/1241))
- feat(ui): add org setup page shown after org creation ([#1257](https://github.com/everruns/everruns/pull/1257))
- feat(ui): add MCP connect button to sidebar header ([#1240](https://github.com/everruns/everruns/pull/1240))
- feat(ui): align harness display_name with agent pattern ([#1246](https://github.com/everruns/everruns/pull/1246))
- fix(auth): resolve org from DB instead of stale JWT in ResolvedOrg ([#1256](https://github.com/everruns/everruns/pull/1256))
- fix(ci): repair SDK compat, CLI E2E tests and build gate ([#1266](https://github.com/everruns/everruns/pull/1266))
- fix(ci): remove redundant version from generated Homebrew formula ([#1237](https://github.com/everruns/everruns/pull/1237))
- fix(ci): wrap linux url/sha256 in CPU conditional ([#1238](https://github.com/everruns/everruns/pull/1238))
- refactor(agents): use "import" verb and unify import endpoint ([#1243](https://github.com/everruns/everruns/pull/1243))
- refactor(core): extract AgentConfigOverlay for composable config merging ([#1244](https://github.com/everruns/everruns/pull/1244))
- refactor(auth): extract API key CRUD routes from auth_routes() ([#1253](https://github.com/everruns/everruns/pull/1253))
- refactor(ui): extract design-system.css from globals.css ([#1254](https://github.com/everruns/everruns/pull/1254))
- refactor(ui): redesign default tool result rendering ([#1245](https://github.com/everruns/everruns/pull/1245))
- test(durable): add agent execution reliability tests ([#1252](https://github.com/everruns/everruns/pull/1252))
- test(ui): add API key org binding test case ([#1258](https://github.com/everruns/everruns/pull/1258))
- test(ui): update test cases for addressable naming ([#1242](https://github.com/everruns/everruns/pull/1242))
- docs: redraw all diagrams as hand-authored SVGs per diagrams spec ([#1236](https://github.com/everruns/everruns/pull/1236))
- chore(specs): document migrations approach ([#1267](https://github.com/everruns/everruns/pull/1267))
- chore(specs): document eval post field and update run execution flow ([#1247](https://github.com/everruns/everruns/pull/1247))
- chore(deps): bump the npm_and_yarn group across 2 directories with 3 updates ([#1251](https://github.com/everruns/everruns/pull/1251))
- chore(deps): bump the npm_and_yarn group across 2 directories with 2 updates ([#1193](https://github.com/everruns/everruns/pull/1193))

### Migration Notes

Migrations squashed: `011_evals_target.sql` + `012_evals_post.sql` → `011_v0.8.10.sql`. In-place upgrade from 0.8.9 works — the squashed migration applies on top of existing `010_v0.8.9` as a normal incremental migration.

## [0.8.9] - 2026-04-08

### Highlights

- **Budgeting System** — Extensible metering, rules, soft enforcement, real `check_budget` tool via gRPC, CLI `--budget-limit`, and MCP support ([#1146](https://github.com/everruns/everruns/pull/1146), [#1208](https://github.com/everruns/everruns/pull/1208), [#1187](https://github.com/everruns/everruns/pull/1187))
- **NATS Pub/Sub for Event Delivery** — Ephemeral events skip PostgreSQL via push-based SSE, NATS pub/sub for task notifications, fire-and-forget gRPC with safe PG fallback ([#1147](https://github.com/everruns/everruns/pull/1147), [#1159](https://github.com/everruns/everruns/pull/1159), [#1161](https://github.com/everruns/everruns/pull/1161), [#1156](https://github.com/everruns/everruns/pull/1156))
- **Tool Output Optimizations** — Tree-sitter structural outlines, content-type-aware read defaults, output verbosity controls, priority-aware truncation, persistent exec output, and hard-limit safety net ([#1199](https://github.com/everruns/everruns/pull/1199), [#1196](https://github.com/everruns/everruns/pull/1196), [#1195](https://github.com/everruns/everruns/pull/1195), [#1197](https://github.com/everruns/everruns/pull/1197), [#1153](https://github.com/everruns/everruns/pull/1153))
- **Network Access Lists** — URL allowlist/blocklist per harness, agent, or session with Ed25519 request signing for HTTP-capable kits ([#1185](https://github.com/everruns/everruns/pull/1185), [#1172](https://github.com/everruns/everruns/pull/1172))
- **Daytona Integration Stabilization** — New `daytona_list_snapshots` and `daytona_api_call` tools, heartbeat fixes, diagnostic hints for signal exits, exec timeout bump to 5m ([#1202](https://github.com/everruns/everruns/pull/1202), [#1206](https://github.com/everruns/everruns/pull/1206), [#1207](https://github.com/everruns/everruns/pull/1207), [#1139](https://github.com/everruns/everruns/pull/1139))

### What's Changed

- feat(budgeting): extensible budgeting system with metering, rules, and agent awareness ([#1146](https://github.com/everruns/everruns/pull/1146)) by [@chaliy](https://github.com/chaliy)
- feat(budgeting): implement real check_budget tool via gRPC ([#1208](https://github.com/everruns/everruns/pull/1208)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add --budget-limit to sessions create, budget support in MCP ([#1187](https://github.com/everruns/everruns/pull/1187)) by [@chaliy](https://github.com/chaliy)
- feat(server): EventDelivery abstraction — ephemeral events skip PG, push-based SSE ([#1147](https://github.com/everruns/everruns/pull/1147)) by [@chaliy](https://github.com/chaliy)
- feat(server): NATS pub/sub for task notifications (Phase 3) ([#1159](https://github.com/everruns/everruns/pull/1159)) by [@chaliy](https://github.com/chaliy)
- feat: NATS end-to-end operational readiness (Phase 3.5) ([#1161](https://github.com/everruns/everruns/pull/1161)) by [@chaliy](https://github.com/chaliy)
- feat(worker): fire-and-forget gRPC for ephemeral deltas, safe PG fallback ([#1156](https://github.com/everruns/everruns/pull/1156)) by [@chaliy](https://github.com/chaliy)
- feat(core): tree-sitter structural outlines for read_file ([#1199](https://github.com/everruns/everruns/pull/1199)) by [@chaliy](https://github.com/chaliy)
- feat(core): content-type-aware read defaults for read_file (EVE-249) ([#1196](https://github.com/everruns/everruns/pull/1196)) by [@chaliy](https://github.com/chaliy)
- feat(core): add output verbosity parameter to all exec tools ([#1195](https://github.com/everruns/everruns/pull/1195)) by [@chaliy](https://github.com/chaliy)
- feat(core): persist exec output to /.outputs/ with separate stdout/stderr (EVE-245) ([#1197](https://github.com/everruns/everruns/pull/1197)) by [@chaliy](https://github.com/chaliy)
- feat(core): add OutputHardLimitHook for tool result size safety net ([#1153](https://github.com/everruns/everruns/pull/1153)) by [@chaliy](https://github.com/chaliy)
- feat(core): add PostToolExecHook and tool output persistence (EVE-222) ([#1148](https://github.com/everruns/everruns/pull/1148)) by [@chaliy](https://github.com/chaliy)
- feat(core): add output-economy hints to exec tool system prompts (EVE-223) ([#1150](https://github.com/everruns/everruns/pull/1150)) by [@chaliy](https://github.com/chaliy)
- feat(core): add system prompt guidance for read_file economy ([#1186](https://github.com/everruns/everruns/pull/1186)) by [@chaliy](https://github.com/chaliy)
- feat(core): add offset/limit pagination to read_file ([#1184](https://github.com/everruns/everruns/pull/1184)) by [@chaliy](https://github.com/chaliy)
- feat(core): add tool output sanitizer for exec tools (EVE-221) ([#1138](https://github.com/everruns/everruns/pull/1138)) by [@chaliy](https://github.com/chaliy)
- feat(core): priority-aware truncation for exec output by [@chaliy](https://github.com/chaliy)
- feat(core): add network access list for harness, agent, session ([#1185](https://github.com/everruns/everruns/pull/1185)) by [@chaliy](https://github.com/chaliy)
- feat(web_fetch): add Ed25519 request signing and key discovery ([#1172](https://github.com/everruns/everruns/pull/1172)) by [@chaliy](https://github.com/chaliy)
- feat(server): add tool_output_persistence to Generic harness ([#1194](https://github.com/everruns/everruns/pull/1194)) by [@chaliy](https://github.com/chaliy)
- feat(core): capture bashkit ToolOutputMetadata for observability (EVE-240) ([#1182](https://github.com/everruns/everruns/pull/1182)) by [@chaliy](https://github.com/chaliy)
- feat(core): add bashkit native cancellation support by [@chaliy](https://github.com/chaliy)
- feat(core): wire session locale to bashkit builder by [@chaliy](https://github.com/chaliy)
- feat(core): operation-based tool narration via narration_noun hint ([#1221](https://github.com/everruns/everruns/pull/1221)) by [@chaliy](https://github.com/chaliy)
- feat(core): add loop detection capability via MessageFilterProvider ([#1154](https://github.com/everruns/everruns/pull/1154)) by [@chaliy](https://github.com/chaliy)
- feat(core): make max_iterations configurable per agent/session ([#1165](https://github.com/everruns/everruns/pull/1165)) by [@chaliy](https://github.com/chaliy)
- feat(compaction): lower keep_recent_tool_outputs default from 5 to 2 (EVE-224) ([#1152](https://github.com/everruns/everruns/pull/1152)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add addressable name for harnesses ([#1209](https://github.com/everruns/everruns/pull/1209)) by [@chaliy](https://github.com/chaliy)
- feat(harness): expose stable name in API, CLI, and OpenAPI ([#1213](https://github.com/everruns/everruns/pull/1213)) by [@chaliy](https://github.com/chaliy)
- feat(harnesses): add interactive name availability check ([#1228](https://github.com/everruns/everruns/pull/1228)) by [@chaliy](https://github.com/chaliy)
- feat(harnesses): virtual default alias for harness lookup by [@chaliy](https://github.com/chaliy)
- feat(harnesses): reserve default in harness name validation by [@chaliy](https://github.com/chaliy)
- feat(agents): add addressable name for agents ([#1229](https://github.com/everruns/everruns/pull/1229)) by [@chaliy](https://github.com/chaliy)
- feat(agents): wire addressable names through UI ([#1231](https://github.com/everruns/everruns/pull/1231)) by [@chaliy](https://github.com/chaliy)
- feat(orgs): accept harness name when setting org default by [@chaliy](https://github.com/chaliy)
- feat(daytona): add daytona_list_snapshots tool ([#1202](https://github.com/everruns/everruns/pull/1202)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): add opt-in daytona_api_call tool for direct API access ([#1206](https://github.com/everruns/everruns/pull/1206)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add JSONL export endpoint, CLI command, and UI button ([#1181](https://github.com/everruns/everruns/pull/1181)) by [@chaliy](https://github.com/chaliy)
- feat(server): add audit logging for SaaS management ops ([#1176](https://github.com/everruns/everruns/pull/1176)) by [@chaliy](https://github.com/chaliy)
- feat(audit): add audit logging abstraction with domains and AOP macro (EVE-226) by [@chaliy](https://github.com/chaliy)
- feat(cli): add --secret flag for session-scoped secrets ([#1144](https://github.com/everruns/everruns/pull/1144)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add connections command for headless API key management ([#1143](https://github.com/everruns/everruns/pull/1143)) by [@chaliy](https://github.com/chaliy)
- feat(cli): support initial_files glob patterns in agent frontmatter ([#1220](https://github.com/everruns/everruns/pull/1220)) by [@chaliy](https://github.com/chaliy)
- feat(ui): replace thinking indicator with combo wave bar ([#1201](https://github.com/everruns/everruns/pull/1201)) by [@chaliy](https://github.com/chaliy)
- feat(ui): expand Ctrl+K search with Evals, Apps, Agent Identities ([#1216](https://github.com/everruns/everruns/pull/1216)) by [@chaliy](https://github.com/chaliy)
- feat(events): emit file.written SSE event on session file create/update ([#1162](https://github.com/everruns/everruns/pull/1162)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): persist cookies across REST-mode tool calls ([#1169](https://github.com/everruns/everruns/pull/1169)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): configurable API and WebSocket base URLs ([#1166](https://github.com/everruns/everruns/pull/1166)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): support returning both screenshot and content from browserless_interact ([#1163](https://github.com/everruns/everruns/pull/1163)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use separate session for heartbeat probes (EVE-255) ([#1207](https://github.com/everruns/everruns/pull/1207)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): wrap exec commands in subshell to prevent session termination ([#1204](https://github.com/everruns/everruns/pull/1204)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): add diagnostic hints for signal-based exit codes ([#1205](https://github.com/everruns/everruns/pull/1205)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): bump default exec timeout from 2m to 5m ([#1139](https://github.com/everruns/everruns/pull/1139)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): auto-source shell profiles before every exec command ([#1142](https://github.com/everruns/everruns/pull/1142)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): increase stale-session threshold for long builds ([#1134](https://github.com/everruns/everruns/pull/1134)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): surface tool defaults in descriptions ([#1155](https://github.com/everruns/everruns/pull/1155)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix streaming cursor artifact rendering on own line ([#1234](https://github.com/everruns/everruns/pull/1234)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix list key props in agent pages ([#1233](https://github.com/everruns/everruns/pull/1233)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix streaming cursor artifact on next line ([#1230](https://github.com/everruns/everruns/pull/1230)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix chat message ordering and input typing performance ([#1222](https://github.com/everruns/everruns/pull/1222)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show friendly label in parent harness select ([#1218](https://github.com/everruns/everruns/pull/1218)) by [@chaliy](https://github.com/chaliy)
- fix(ui): filter model selectors to only show installed models ([#1217](https://github.com/everruns/everruns/pull/1217)) by [@chaliy](https://github.com/chaliy)
- fix(agents): enforce addressable name validation across all creation paths ([#1232](https://github.com/everruns/everruns/pull/1232)) by [@chaliy](https://github.com/chaliy)
- fix(server): publish input.message events to shared EventDelivery ([#1225](https://github.com/everruns/everruns/pull/1225)) by [@chaliy](https://github.com/chaliy)
- fix(server): look up agents by public_id, not internal UUID ([#1178](https://github.com/everruns/everruns/pull/1178)) by [@chaliy](https://github.com/chaliy)
- fix(core): empty id/search crash in read_capabilities and tool error logging ([#1215](https://github.com/everruns/everruns/pull/1215)) by [@chaliy](https://github.com/chaliy)
- fix(core): skip AGENTS.md read on message-filter-only path ([#1164](https://github.com/everruns/everruns/pull/1164)) by [@chaliy](https://github.com/chaliy)
- fix(core): delegate bash tool input_schema() to bashkit by [@chaliy](https://github.com/chaliy)
- fix(tools): disambiguate session read_file from sandbox file tools ([#1203](https://github.com/everruns/everruns/pull/1203)) by [@chaliy](https://github.com/chaliy)
- fix(sessions): default to Generic harness when no harness_id specified ([#1179](https://github.com/everruns/everruns/pull/1179)) by [@chaliy](https://github.com/chaliy)
- fix(cli): replace raw reqwest budget calls with SDK ([#1200](https://github.com/everruns/everruns/pull/1200)) by [@chaliy](https://github.com/chaliy)
- fix(cli): use SDK for connections remove, document model gaps ([#1175](https://github.com/everruns/everruns/pull/1175)) by [@chaliy](https://github.com/chaliy)
- fix(cli): use SDK set_secrets instead of raw reqwest ([#1174](https://github.com/everruns/everruns/pull/1174)) by [@chaliy](https://github.com/chaliy)
- fix(llm-drivers): use model profile max_output_tokens instead of hardcoded defaults ([#1160](https://github.com/everruns/everruns/pull/1160)) by [@chaliy](https://github.com/chaliy)
- fix(worker): in-turn message steering for mid-turn user messages ([#1137](https://github.com/everruns/everruns/pull/1137)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): surface clear error when CDP returns 403 ([#1167](https://github.com/everruns/everruns/pull/1167)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): set WORKER_GRPC_AUTH_TOKEN in start-all mode ([#1135](https://github.com/everruns/everruns/pull/1135)) by [@chaliy](https://github.com/chaliy)
- fix(example): use ADDR env var so server listens on correct port ([#1145](https://github.com/everruns/everruns/pull/1145)) by [@chaliy](https://github.com/chaliy)
- fix(ci): replace deprecated macos-13 runner with macos-latest ([#1214](https://github.com/everruns/everruns/pull/1214)) by [@chaliy](https://github.com/chaliy)
- fix(docs): fix code block background colors and upgrade deps ([#1177](https://github.com/everruns/everruns/pull/1177)) by [@chaliy](https://github.com/chaliy)
- fix(docs): improve code block syntax highlighting ([#1158](https://github.com/everruns/everruns/pull/1158)) by [@chaliy](https://github.com/chaliy)
- refactor(core): extract persist_large_output helper and annotate truncated output ([#1188](https://github.com/everruns/everruns/pull/1188)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): use shared selector components in form pages ([#1219](https://github.com/everruns/everruns/pull/1219)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash migrations 010-014 into v0.8.9 ([#1212](https://github.com/everruns/everruns/pull/1212)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump bashkit v0.1.14 → v0.1.15, adopt max_memory ([#1210](https://github.com/everruns/everruns/pull/1210)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump sdk, bashkit, and fetchkit ([#1171](https://github.com/everruns/everruns/pull/1171)) by [@chaliy](https://github.com/chaliy)
- chore: add .npmrc with ignore-scripts for supply-chain hardening ([#1170](https://github.com/everruns/everruns/pull/1170)) by [@chaliy](https://github.com/chaliy)
- chore: add nats-server to just init (setup.sh) ([#1168](https://github.com/everruns/everruns/pull/1168)) by [@chaliy](https://github.com/chaliy)
- chore(build): remove sccache integration ([#1151](https://github.com/everruns/everruns/pull/1151)) by [@chaliy](https://github.com/chaliy)
- feat(examples): add VictoriaMetrics to full docker-compose example ([#1149](https://github.com/everruns/everruns/pull/1149)) by [@chaliy](https://github.com/chaliy)
- docs: advanced guide for read-category tools and context economy ([#1198](https://github.com/everruns/everruns/pull/1198)) by [@chaliy](https://github.com/chaliy)
- docs: update context management docs for Generic harness ([#1136](https://github.com/everruns/everruns/pull/1136)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.8 → 0.8.9:** Migrations squashed into v0.8.9 ([#1212](https://github.com/everruns/everruns/pull/1212)). Fresh database required.

## [0.8.8] - 2026-03-28

### Highlights

- **Everruns as MCP Server** — ScriptedTool-backed MCP endpoint with OAuth 2.1 PKCE authentication ([#1078](https://github.com/everruns/everruns/pull/1078), [#1092](https://github.com/everruns/everruns/pull/1092))
- **Persistent Memory Layer** — Cross-session memory capability for agents ([#1091](https://github.com/everruns/everruns/pull/1091))
- **New Sandbox Integrations** — Added E2B, Deno Deploy, Sprites, and PI sandbox providers ([#1038](https://github.com/everruns/everruns/pull/1038), [#1047](https://github.com/everruns/everruns/pull/1047), [#1076](https://github.com/everruns/everruns/pull/1076), [#1085](https://github.com/everruns/everruns/pull/1085))
- **API Hardening** — Per-IP rate limiting, configurable resource limits, error sanitization, and account deletion/export ([#1117](https://github.com/everruns/everruns/pull/1117), [#1119](https://github.com/everruns/everruns/pull/1119), [#1116](https://github.com/everruns/everruns/pull/1116), [#1123](https://github.com/everruns/everruns/pull/1123))
- **Prometheus /metrics** — Production-ready metrics endpoint with horizontal scaling support ([#1101](https://github.com/everruns/everruns/pull/1101), [#1106](https://github.com/everruns/everruns/pull/1106))
- **Started Work on Evals Subsystem** — User-facing eval system for agents and harnesses, gated behind experimental feature flag ([#1121](https://github.com/everruns/everruns/pull/1121), [#1122](https://github.com/everruns/everruns/pull/1122))

### What's Changed

- feat(mcp): ScriptedTool-backed MCP endpoint at /mcp ([#1078](https://github.com/everruns/everruns/pull/1078)) by [@chaliy](https://github.com/chaliy)
- feat(auth): add MCP OAuth 2.1 with PKCE for MCP client authentication ([#1092](https://github.com/everruns/everruns/pull/1092)) by [@chaliy](https://github.com/chaliy)
- feat(memory): add persistent cross-session memory capability ([#1091](https://github.com/everruns/everruns/pull/1091)) by [@chaliy](https://github.com/chaliy)
- feat(e2b): add cloud sandbox integration ([#1038](https://github.com/everruns/everruns/pull/1038)) by [@chaliy](https://github.com/chaliy)
- feat(deno): add Deno Deploy sandbox integration ([#1047](https://github.com/everruns/everruns/pull/1047)) by [@chaliy](https://github.com/chaliy)
- feat(deno): bring Deno integration to parity with Daytona ([#1052](https://github.com/everruns/everruns/pull/1052)) by [@chaliy](https://github.com/chaliy)
- feat(sprites): add Sprites sandbox integration ([#1076](https://github.com/everruns/everruns/pull/1076)) by [@chaliy](https://github.com/chaliy)
- feat(pi): add PI sandbox coding agent capability ([#1085](https://github.com/everruns/everruns/pull/1085)) by [@chaliy](https://github.com/chaliy)
- feat(server): add Prometheus /metrics endpoint ([#1101](https://github.com/everruns/everruns/pull/1101)) by [@chaliy](https://github.com/chaliy)
- feat(evals): add user-facing eval system for agents and harnesses ([#1121](https://github.com/everruns/everruns/pull/1121)) by [@chaliy](https://github.com/chaliy)
- feat(evals): gate evals behind experimental feature flag ([#1122](https://github.com/everruns/everruns/pull/1122)) by [@chaliy](https://github.com/chaliy)
- feat(server): add account deletion and data export endpoints ([#1123](https://github.com/everruns/everruns/pull/1123)) by [@chaliy](https://github.com/chaliy)
- feat(api): add global per-IP API rate limiting middleware ([#1119](https://github.com/everruns/everruns/pull/1119)) by [@chaliy](https://github.com/chaliy)
- feat(api): add configurable resource limits for orgs, members, API keys ([#1117](https://github.com/everruns/everruns/pull/1117)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add custom error pages (404, 500) ([#1118](https://github.com/everruns/everruns/pull/1118)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add mermaid diagram rendering to chat messages ([#1073](https://github.com/everruns/everruns/pull/1073)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add --writable flag for initial-files-dir ([#1128](https://github.com/everruns/everruns/pull/1128)) by [@chaliy](https://github.com/chaliy)
- feat(cli): rename .syncignore to .everrunsignore ([#1104](https://github.com/everruns/everruns/pull/1104)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add sessions watch command for real-time monitoring ([#1046](https://github.com/everruns/everruns/pull/1046)) by [@chaliy](https://github.com/chaliy)
- feat(cli): remove default timeout from chat command ([#1044](https://github.com/everruns/everruns/pull/1044)) by [@chaliy](https://github.com/chaliy)
- feat(harness): enable compaction by default on Generic harness ([#1126](https://github.com/everruns/everruns/pull/1126)) by [@chaliy](https://github.com/chaliy)
- feat(apps): support multiple channels per app ([#1088](https://github.com/everruns/everruns/pull/1088)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add session-level system_prompt and initial_files overrides ([#1095](https://github.com/everruns/everruns/pull/1095)) by [@chaliy](https://github.com/chaliy)
- feat(core): add multi-platform channel abstractions ([#1080](https://github.com/everruns/everruns/pull/1080)) by [@chaliy](https://github.com/chaliy)
- feat(core): add ToolHints to tool definitions ([#1074](https://github.com/everruns/everruns/pull/1074)) by [@chaliy](https://github.com/chaliy)
- feat(events): add tool.output.delta for streamed tool output ([#1086](https://github.com/everruns/everruns/pull/1086)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): stream exec output in real time via tool.output.delta ([#1096](https://github.com/everruns/everruns/pull/1096)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add tool.progress streaming for status feedback ([#1051](https://github.com/everruns/everruns/pull/1051)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add secret references in interact steps ([#1042](https://github.com/everruns/everruns/pull/1042)) by [@chaliy](https://github.com/chaliy)
- feat(blueprints): implement agent blueprints infrastructure ([#1055](https://github.com/everruns/everruns/pull/1055)) by [@chaliy](https://github.com/chaliy)
- feat(agent-identities): add identity-scoped connections ([#1034](https://github.com/everruns/everruns/pull/1034)) by [@chaliy](https://github.com/chaliy)
- feat(server): resolve connections from agent identity on session ([#1039](https://github.com/everruns/everruns/pull/1039)) by [@chaliy](https://github.com/chaliy)
- feat(cli): restore --initial-files-dir flag for agents create/update ([#1064](https://github.com/everruns/everruns/pull/1064)) by [@chaliy](https://github.com/chaliy)
- feat(core): add link-following hint to agent_instructions prompt ([e6ac5e28](https://github.com/everruns/everruns/commit/e6ac5e28)) by [@chaliy](https://github.com/chaliy)
- feat(agent-identity): align edit page with agent edit patterns and centralize locale/timezone ([7564d59d](https://github.com/everruns/everruns/commit/7564d59d)) by [@chaliy](https://github.com/chaliy)
- feat(deno): support personal tokens (ddp_...) in generic connection flow ([#1063](https://github.com/everruns/everruns/pull/1063)) by [@chaliy](https://github.com/chaliy)
- feat(ci): build server and worker binaries for Linux releases ([#1120](https://github.com/everruns/everruns/pull/1120)) by [@chaliy](https://github.com/chaliy)
- feat(ci): auto-update Homebrew formula after CLI releases ([#1090](https://github.com/everruns/everruns/pull/1090)) by [@chaliy](https://github.com/chaliy)
- feat(ci): add Sprites integration workflow with live API tests ([#1077](https://github.com/everruns/everruns/pull/1077)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): detect dead session shell and auto-recover ([#1129](https://github.com/everruns/everruns/pull/1129)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): normalize session paths in download_workspace ([#1127](https://github.com/everruns/everruns/pull/1127)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): handle empty path/branch and quote shell args in git_clone ([#1125](https://github.com/everruns/everruns/pull/1125)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use proper encoding for binary files in download_workspace ([#1124](https://github.com/everruns/everruns/pull/1124)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): migrate exec to Session API with unified streaming ([#1108](https://github.com/everruns/everruns/pull/1108)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): wrap exec polling commands in sh -c for shell redirection ([#1105](https://github.com/everruns/everruns/pull/1105)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use snapshot-based sizing instead of ignored resource params ([#1099](https://github.com/everruns/everruns/pull/1099)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): auto-renew sandbox lease during long-running exec ([#1087](https://github.com/everruns/everruns/pull/1087)) by [@chaliy](https://github.com/chaliy)
- fix(api): sanitize error responses to prevent internal detail leaks ([#1116](https://github.com/everruns/everruns/pull/1116)) by [@chaliy](https://github.com/chaliy)
- fix(deno): force HTTP/1.1 ALPN and add proxy auth for WebSocket ([#1114](https://github.com/everruns/everruns/pull/1114)) by [@chaliy](https://github.com/chaliy)
- fix(auth): return clear error when CLI login user has no orgs ([#1115](https://github.com/everruns/everruns/pull/1115)) by [@chaliy](https://github.com/chaliy)
- fix(cli): allow .agents/ directory in --initial-files-dir ([#1110](https://github.com/everruns/everruns/pull/1110)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): resolve dependencies in collect_capabilities ([#1113](https://github.com/everruns/everruns/pull/1113)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): use /chromium path, force HTTP/1.1 ALPN, add proxy support for CDP WebSocket ([#1112](https://github.com/everruns/everruns/pull/1112)) by [@chaliy](https://github.com/chaliy)
- fix(server): make Prometheus metrics correct under horizontal scaling ([#1106](https://github.com/everruns/everruns/pull/1106)) by [@chaliy](https://github.com/chaliy)
- fix(cli): replace chat polling with SSE streaming and efficient snapshot ([#1094](https://github.com/everruns/everruns/pull/1094)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove duplicate Connections header on agent identity page ([#1109](https://github.com/everruns/everruns/pull/1109)) by [@chaliy](https://github.com/chaliy)
- fix(ui): improve code block styling in light theme ([#1072](https://github.com/everruns/everruns/pull/1072)) by [@chaliy](https://github.com/chaliy)
- fix(ui): widen API key dialog and inline copy button ([#1035](https://github.com/everruns/everruns/pull/1035)) by [@chaliy](https://github.com/chaliy)
- fix(ui,docs): fix code block empty header bar and background contrast ([#1083](https://github.com/everruns/everruns/pull/1083)) by [@chaliy](https://github.com/chaliy)
- fix(worker): prevent concurrent turn corruption (EVE-170) ([#1037](https://github.com/everruns/everruns/pull/1037)) by [@chaliy](https://github.com/chaliy)
- fix(durable): notify workflow when reclaimed tasks are marked dead ([#1061](https://github.com/everruns/everruns/pull/1061)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): add timeouts to CdpSession connect and send_command ([#1059](https://github.com/everruns/everruns/pull/1059)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): accept HTTP 204 from v2 /active endpoint ([#1036](https://github.com/everruns/everruns/pull/1036)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): use correct env var name BROWSERLESS_TOKEN ([#1068](https://github.com/everruns/everruns/pull/1068)) by [@chaliy](https://github.com/chaliy)
- fix(e2b): align response structs with current E2B API format ([#1054](https://github.com/everruns/everruns/pull/1054)) by [@chaliy](https://github.com/chaliy)
- fix(server): include initial_files in agent upsert SQL query ([#1069](https://github.com/everruns/everruns/pull/1069)) by [@chaliy](https://github.com/chaliy)
- fix(images): use SessionId instead of Uuid for upload query parameter ([#1049](https://github.com/everruns/everruns/pull/1049)) by [@chaliy](https://github.com/chaliy)
- fix(cli): robust event filtering in chat polling ([#1062](https://github.com/everruns/everruns/pull/1062)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): remove invalid `local` outside function in setup.sh ([#1093](https://github.com/everruns/everruns/pull/1093)) by [@chaliy](https://github.com/chaliy)
- fix(docs): fix mermaid diagram rendering in docs site ([#1075](https://github.com/everruns/everruns/pull/1075)) by [@chaliy](https://github.com/chaliy)
- fix(core): install rustls CryptoProvider at startup for parallel tool execution ([b1c19357](https://github.com/everruns/everruns/commit/b1c19357)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): declare session_storage as dependency ([b2b360b4](https://github.com/everruns/everruns/commit/b2b360b4)) by [@chaliy](https://github.com/chaliy)
- fix: rustls CryptoProvider in e2b test + renumber duplicate migration ([#1067](https://github.com/everruns/everruns/pull/1067)) by [@chaliy](https://github.com/chaliy)
- fix(ci): replace deprecated macos-13 runner with macos-latest ([#1033](https://github.com/everruns/everruns/pull/1033)) by [@chaliy](https://github.com/chaliy)
- refactor(auth): decouple CLI auth routes from BuiltinAuthBackend (EVE-176) ([#1050](https://github.com/everruns/everruns/pull/1050)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): split platform management tools into read/write ([#1070](https://github.com/everruns/everruns/pull/1070)) by [@chaliy](https://github.com/chaliy)
- refactor(seed): move multi-org harness reconciliation to non-blocking background task ([#1082](https://github.com/everruns/everruns/pull/1082)) by [@chaliy](https://github.com/chaliy)
- refactor(skills): remove redundant bundled_files from activate_skill result ([#1040](https://github.com/everruns/everruns/pull/1040)) by [@chaliy](https://github.com/chaliy)
- perf(core): cache compiled regexes in skill.rs with LazyLock ([ad5f11e4](https://github.com/everruns/everruns/commit/ad5f11e4)) by [@chaliy](https://github.com/chaliy)
- test(mcp): add integration tests for MCP endpoint ([#1100](https://github.com/everruns/everruns/pull/1100)) by [@chaliy](https://github.com/chaliy)
- test(e2b): add integration tests and CI for E2B cloud sandbox ([#1048](https://github.com/everruns/everruns/pull/1048)) by [@chaliy](https://github.com/chaliy)
- test(browserless): add CI jobs for browserless integration tests ([#1053](https://github.com/everruns/everruns/pull/1053)) by [@chaliy](https://github.com/chaliy)
- test(agent-chat): add multi-turn conversation UI test case ([#1097](https://github.com/everruns/everruns/pull/1097)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash v0.8.8 SQL migrations ([#1131](https://github.com/everruns/everruns/pull/1131)) by [@chaliy](https://github.com/chaliy)
- chore(deps): fix npm vulnerabilities and update deps ([#1130](https://github.com/everruns/everruns/pull/1130)) by [@chaliy](https://github.com/chaliy)
- chore(deps): upgrade notify 7→8.2, reqwest 0.13.1→0.13.2 ([#1043](https://github.com/everruns/everruns/pull/1043)) by [@chaliy](https://github.com/chaliy)
- chore: remove protoc build dependency ([#1079](https://github.com/everruns/everruns/pull/1079)) by [@chaliy](https://github.com/chaliy)
- chore(specs): add Figma design system reference to brand spec ([#1111](https://github.com/everruns/everruns/pull/1111)) by [@chaliy](https://github.com/chaliy)
- chore(specs): merge mcp-oauth.md into mcp.md ([#1103](https://github.com/everruns/everruns/pull/1103)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove implementation details that duplicate code ([#1089](https://github.com/everruns/everruns/pull/1089)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove temporary analysis, clarify durable memory principle ([#1081](https://github.com/everruns/everruns/pull/1081)) by [@chaliy](https://github.com/chaliy)
- chore(test-cases): fix structure, numbering, and spec compliance ([#1098](https://github.com/everruns/everruns/pull/1098)) by [@chaliy](https://github.com/chaliy)
- docs(daytona): mention all autocleanup timeouts in system prompt ([#1084](https://github.com/everruns/everruns/pull/1084)) by [@chaliy](https://github.com/chaliy)
- docs(cli): add Homebrew installation instructions ([#1060](https://github.com/everruns/everruns/pull/1060)) by [@chaliy](https://github.com/chaliy)

## [0.8.7] - 2026-03-22

### Highlights

- **Agent Identities** — Virtual principals for unattended execution across backend, API, DB, and UI ([#1029](https://github.com/everruns/everruns/pull/1029))
- **CLI + Interactive Login** — Install script, interactive OAuth login, file sync commands, and pre-built release binaries ([#1013](https://github.com/everruns/everruns/pull/1013), [#969](https://github.com/everruns/everruns/pull/969), [#968](https://github.com/everruns/everruns/pull/968), [#1000](https://github.com/everruns/everruns/pull/1000))
- **Session Filesystem** — Git version control for session files with hash-gated edit_file tool ([#979](https://github.com/everruns/everruns/pull/979), [#942](https://github.com/everruns/everruns/pull/942))
- **Localization** — Full Ukrainian chat UI coverage ([#1005](https://github.com/everruns/everruns/pull/1005))

### What's Changed

- feat(agent-identities): add agent identities across backend, API, DB, and UI ([#1029](https://github.com/everruns/everruns/pull/1029)) by [@chaliy](https://github.com/chaliy)
- feat(worker): increase default max concurrent tasks from 10 to 1000 ([#1027](https://github.com/everruns/everruns/pull/1027)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): expose cpu, memory, and disk resource options on sandbox creation ([#1024](https://github.com/everruns/everruns/pull/1024)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): add auto-archive and auto-delete lifecycle settings ([#1026](https://github.com/everruns/everruns/pull/1026)) by [@chaliy](https://github.com/chaliy)
- feat(core): protect skill content from context compaction ([#1022](https://github.com/everruns/everruns/pull/1022)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add auth modes and OAuth connection flow ([#1018](https://github.com/everruns/everruns/pull/1018)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add agents update command and --initial-files-dir flag ([#1020](https://github.com/everruns/everruns/pull/1020)) by [@chaliy](https://github.com/chaliy)
- feat(core): implement SearchCapable for bashkit indexed search ([#1014](https://github.com/everruns/everruns/pull/1014)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add install script and just recipe ([#1013](https://github.com/everruns/everruns/pull/1013)) by [@chaliy](https://github.com/chaliy)
- feat(agents): upsert on import when agent ID exists ([#1010](https://github.com/everruns/everruns/pull/1010)) by [@chaliy](https://github.com/chaliy)
- feat(ci): publish pre-built CLI binaries to GitHub releases ([#1000](https://github.com/everruns/everruns/pull/1000)) by [@chaliy](https://github.com/chaliy)
- feat(server): add git version control for session filesystems ([#979](https://github.com/everruns/everruns/pull/979)) by [@chaliy](https://github.com/chaliy)
- feat(server): seed example agents during org init ([#985](https://github.com/everruns/everruns/pull/985)) by [@chaliy](https://github.com/chaliy)
- feat(cli): interactive login with localhost OAuth callback ([#969](https://github.com/everruns/everruns/pull/969)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add ${SESSION_ID} and ${SKILL_DIR} variable substitution ([#974](https://github.com/everruns/everruns/pull/974)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add file sync commands and comprehensive test coverage ([#968](https://github.com/everruns/everruns/pull/968)) by [@chaliy](https://github.com/chaliy)
- feat(core): add client hints mechanism and gate setup_connection ([#b04d6e28](https://github.com/everruns/everruns/commit/b04d6e28)) by [@chaliy](https://github.com/chaliy)
- feat(server): implement 5-minute timeout for waiting_for_tool_results sessions ([#961](https://github.com/everruns/everruns/pull/961)) by [@chaliy](https://github.com/chaliy)
- feat(core): cap tool result size to 64 KiB before sending to LLM ([#953](https://github.com/everruns/everruns/pull/953)) by [@chaliy](https://github.com/chaliy)
- feat(apps): add slack report-progress reply mode ([#954](https://github.com/everruns/everruns/pull/954)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add instruction hierarchy to Generic harness system prompt ([#950](https://github.com/everruns/everruns/pull/950)) by [@chaliy](https://github.com/chaliy)
- feat(session-file-system): add hash-gated edit_file tool ([#942](https://github.com/everruns/everruns/pull/942)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add inheritance and effective previews ([#932](https://github.com/everruns/everruns/pull/932)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add model frontmatter field for per-skill model override ([#934](https://github.com/everruns/everruns/pull/934)) by [@chaliy](https://github.com/chaliy)
- feat(permissions): add skill-scoped permission rules ([#931](https://github.com/everruns/everruns/pull/931)) by [@chaliy](https://github.com/chaliy)
- feat(embedding): add PlatformDefinition for embeddable runtimes ([#929](https://github.com/everruns/everruns/pull/929)) by [@chaliy](https://github.com/chaliy)
- feat(anthropic): adopt model metadata from /v1/models API ([#925](https://github.com/everruns/everruns/pull/925)) by [@chaliy](https://github.com/chaliy)
- feat(core): add GPT-5.4 mini/nano profiles and tiered pricing support ([#927](https://github.com/everruns/everruns/pull/927)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add context: fork and agent frontmatter fields ([#926](https://github.com/everruns/everruns/pull/926)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add dynamic context injection via !command syntax ([#923](https://github.com/everruns/everruns/pull/923)) by [@chaliy](https://github.com/chaliy)
- feat(skills): positional argument substitution ([#914](https://github.com/everruns/everruns/pull/914)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add disable-model-invocation frontmatter field ([#913](https://github.com/everruns/everruns/pull/913)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add manual-ui-testing skill ([#912](https://github.com/everruns/everruns/pull/912)) by [@chaliy](https://github.com/chaliy)
- feat(core): enable Opus 4.6 1M context and add max_media limit ([#890](https://github.com/everruns/everruns/pull/890)) by [@chaliy](https://github.com/chaliy)
- feat(compaction): multi-strategy context compaction ([#883](https://github.com/everruns/everruns/pull/883)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add Twitter/OG social card preview metadata ([#886](https://github.com/everruns/everruns/pull/886)) by [@chaliy](https://github.com/chaliy)
- feat(server): seed admin user at startup in admin auth mode ([#882](https://github.com/everruns/everruns/pull/882)) by [@chaliy](https://github.com/chaliy)
- fix(durable): strip null bytes from JSON before PostgreSQL jsonb insert ([#1031](https://github.com/everruns/everruns/pull/1031)) by [@chaliy](https://github.com/chaliy)
- fix(worker): treat Pending workflow as takeover-safe and cancel stale tasks ([#1025](https://github.com/everruns/everruns/pull/1025)) by [@chaliy](https://github.com/chaliy)
- fix(core): add fallback parsing for malformed SKILL.md YAML frontmatter ([#1021](https://github.com/everruns/everruns/pull/1021)) by [@chaliy](https://github.com/chaliy)
- fix(localization): finish Ukrainian chat UI coverage ([#1005](https://github.com/everruns/everruns/pull/1005)) by [@chaliy](https://github.com/chaliy)
- fix(auth): support API keys via standard Bearer scheme ([#1016](https://github.com/everruns/everruns/pull/1016)) by [@chaliy](https://github.com/chaliy)
- fix(cli): simplify install-cli recipe, fix version parsing ([#1017](https://github.com/everruns/everruns/pull/1017)) by [@chaliy](https://github.com/chaliy)
- fix(cli): fix four CLI bugs — streaming, upsert, capabilities list, optional harness ([#1009](https://github.com/everruns/everruns/pull/1009)) by [@chaliy](https://github.com/chaliy)
- fix(cli): show credentials path in status and fix macOS path docs ([#1008](https://github.com/everruns/everruns/pull/1008)) by [@chaliy](https://github.com/chaliy)
- fix: remove automatic agent seeding to prevent duplicates with examples ([#1004](https://github.com/everruns/everruns/pull/1004)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): replace 150MB gRPC message limit with presigned URLs for images ([#1001](https://github.com/everruns/everruns/pull/1001)) by [@chaliy](https://github.com/chaliy)
- fix(core): make session_interact schema OpenAI-compatible ([#996](https://github.com/everruns/everruns/pull/996)) by [@chaliy](https://github.com/chaliy)
- fix(core): add missing properties to object tool schemas ([#984](https://github.com/everruns/everruns/pull/984)) by [@chaliy](https://github.com/chaliy)
- fix(platform): default harness_id to Generic in manage_sessions ([#982](https://github.com/everruns/everruns/pull/982)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): unify gRPC error handling across 3 crates ([#980](https://github.com/everruns/everruns/pull/980)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate virtual capability references on write ([#981](https://github.com/everruns/everruns/pull/981)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add informative tooltip to chat sidebar warning badge ([#973](https://github.com/everruns/everruns/pull/973)) by [@chaliy](https://github.com/chaliy)
- fix(worker): use per-provider circuit breaker keys ([#971](https://github.com/everruns/everruns/pull/971)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): add GetMessage RPC to replace O(n) message lookup ([#970](https://github.com/everruns/everruns/pull/970)) by [@chaliy](https://github.com/chaliy)
- fix(ui): replace unsafe type casts with type guards ([#967](https://github.com/everruns/everruns/pull/967)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent schedules table horizontal overflow at 1280px viewport ([#4b6fcf4f](https://github.com/everruns/everruns/commit/4b6fcf4f)) by [@chaliy](https://github.com/chaliy)
- fix(core): remove top-level oneOf from edit_file tool schema ([#966](https://github.com/everruns/everruns/pull/966)) by [@chaliy](https://github.com/chaliy)
- fix(ui): make org switching atomic with cookie sync and query invalidation ([#964](https://github.com/everruns/everruns/pull/964)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate initial events REST fetch on chat page load ([#963](https://github.com/everruns/everruns/pull/963)) by [@chaliy](https://github.com/chaliy)
- fix(ui): unify API error handling through centralized client ([#962](https://github.com/everruns/everruns/pull/962)) by [@chaliy](https://github.com/chaliy)
- fix(security): encrypt app channel_config secrets at rest ([#960](https://github.com/everruns/everruns/pull/960)) by [@chaliy](https://github.com/chaliy)
- fix(auth-sync): support authoritative org membership updates and removals ([#952](https://github.com/everruns/everruns/pull/952)) by [@chaliy](https://github.com/chaliy)
- fix(ui): constrain ScrollArea max-height overflow ([#956](https://github.com/everruns/everruns/pull/956)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove chat picker inset chrome ([#947](https://github.com/everruns/everruns/pull/947)) by [@chaliy](https://github.com/chaliy)
- fix(api): return 404 for missing app harness and agent references ([#939](https://github.com/everruns/everruns/pull/939)) by [@chaliy](https://github.com/chaliy)
- fix(worker): skip InputAtom when resuming after connection_required ([#bdb47f47](https://github.com/everruns/everruns/commit/bdb47f47)) by [@chaliy](https://github.com/chaliy)
- fix(core): deduplicate tools by name in RuntimeAgentBuilder ([#946](https://github.com/everruns/everruns/pull/946)) by [@chaliy](https://github.com/chaliy)
- fix(worker): skip retries for non-retryable durable task errors ([#944](https://github.com/everruns/everruns/pull/944)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate utility functions across frontend ([#941](https://github.com/everruns/everruns/pull/941)) by [@chaliy](https://github.com/chaliy)
- fix(api): return 404 for missing harness IDs in update and destroy ([#938](https://github.com/everruns/everruns/pull/938)) by [@chaliy](https://github.com/chaliy)
- fix(api): return empty results for unknown agent_id in session list ([#937](https://github.com/everruns/everruns/pull/937)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate default_model_id in agent upsert ([#935](https://github.com/everruns/everruns/pull/935)) by [@chaliy](https://github.com/chaliy)
- fix(worker): track actual workflow iteration count ([#930](https://github.com/everruns/everruns/pull/930)) by [@chaliy](https://github.com/chaliy)
- fix(auth): resolve org cookie in no-auth mode for multi-org support ([#918](https://github.com/everruns/everruns/pull/918)) by [@chaliy](https://github.com/chaliy)
- fix(auth): update threat model for OAuth state validation ([#922](https://github.com/everruns/everruns/pull/922)) by [@chaliy](https://github.com/chaliy)
- fix(core): deduplicate ModelWithProvider type across crates ([#921](https://github.com/everruns/everruns/pull/921)) by [@chaliy](https://github.com/chaliy)
- fix(core): skip serializing default deferrable policy in tool types ([#920](https://github.com/everruns/everruns/pull/920)) by [@chaliy](https://github.com/chaliy)
- fix(worker): report actual load in heartbeat instead of default ([#919](https://github.com/everruns/everruns/pull/919)) by [@chaliy](https://github.com/chaliy)
- fix(server): downgrade temporary debug logging in event service ([#917](https://github.com/everruns/everruns/pull/917)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show harness names instead of raw IDs in settings dropdowns ([#915](https://github.com/everruns/everruns/pull/915)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix schedule creation form silently failing ([#911](https://github.com/everruns/everruns/pull/911)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add confirmation dialog for MCP server archive ([#906](https://github.com/everruns/everruns/pull/906)) by [@chaliy](https://github.com/chaliy)
- fix(ui): auto-switch to newly created org ([#910](https://github.com/everruns/everruns/pull/910)) by [@chaliy](https://github.com/chaliy)
- fix(ui): refresh org dropdown after creating new org ([#907](https://github.com/everruns/everruns/pull/907)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redirect to setup page after org creation ([#905](https://github.com/everruns/everruns/pull/905)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): handle empty ui_dev_args under set -u ([#902](https://github.com/everruns/everruns/pull/902)) by [@chaliy](https://github.com/chaliy)
- fix: resolve open Dependabot security alerts (Next.js + lodash) ([#899](https://github.com/everruns/everruns/pull/899)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add missing SSE event types for connection setup and compaction ([#897](https://github.com/everruns/everruns/pull/897)) by [@chaliy](https://github.com/chaliy)
- fix(server): allow waiting_for_tool_results in gRPC set_session_status ([#893](https://github.com/everruns/everruns/pull/893)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove native picker chrome ([#894](https://github.com/everruns/everruns/pull/894)) by [@chaliy](https://github.com/chaliy)
- fix(docs): render correct sidebar on API reference pages ([#887](https://github.com/everruns/everruns/pull/887)) by [@chaliy](https://github.com/chaliy)
- fix(ui): align chat surface with runtime previews ([#885](https://github.com/everruns/everruns/pull/885)) by [@chaliy](https://github.com/chaliy)
- fix(core): default system_prompt on create and render links in platform chat ([#884](https://github.com/everruns/everruns/pull/884)) by [@chaliy](https://github.com/chaliy)
- refactor(cli): upgrade deps, drop serde_yaml, use server import API ([#1028](https://github.com/everruns/everruns/pull/1028)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): combine agents page into single view with links to full lists ([#1006](https://github.com/everruns/everruns/pull/1006)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): split monolith files (types.ts, settings, queues) ([#1002](https://github.com/everruns/everruns/pull/1002)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract magic values to named constants ([#999](https://github.com/everruns/everruns/pull/999)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract generic CRUD APIs and hooks ([#994](https://github.com/everruns/everruns/pull/994)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): decompose chat panel and sidebar ([#991](https://github.com/everruns/everruns/pull/991)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract QueryStateWrapper for list page boilerplate ([#945](https://github.com/everruns/everruns/pull/945)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): create centralized TOOL_REGISTRY for tool card polymorphism ([#943](https://github.com/everruns/everruns/pull/943)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): move inline CSS strings to dedicated CSS files ([#949](https://github.com/everruns/everruns/pull/949)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract useScrollManager and useImageDropZone hooks ([#936](https://github.com/everruns/everruns/pull/936)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): replace derived-state useEffects with inline computation ([#889](https://github.com/everruns/everruns/pull/889)) by [@chaliy](https://github.com/chaliy)
- refactor(api): add ApiResult type alias and impl_auth_state! macro ([#998](https://github.com/everruns/everruns/pull/998)) by [@chaliy](https://github.com/chaliy)
- refactor(store): add StoreResultExt trait and JSON helpers ([#997](https://github.com/everruns/everruns/pull/997)) by [@chaliy](https://github.com/chaliy)
- refactor(infra): replace Docker Compose with native pg_ctl + valkey-server ([#995](https://github.com/everruns/everruns/pull/995)) by [@chaliy](https://github.com/chaliy)
- refactor(llm): extract shared LLM driver helpers ([#993](https://github.com/everruns/everruns/pull/993)) by [@chaliy](https://github.com/chaliy)
- refactor: rename agent templates to examples, install to use ([#992](https://github.com/everruns/everruns/pull/992)) by [@chaliy](https://github.com/chaliy)
- refactor(worker): consolidate adapter wrappers ([#989](https://github.com/everruns/everruns/pull/989), [#990](https://github.com/everruns/everruns/pull/990)) by [@chaliy](https://github.com/chaliy)
- refactor(worker): extract domain logic from workers into atoms and shared modules ([#958](https://github.com/everruns/everruns/pull/958)) by [@chaliy](https://github.com/chaliy)
- refactor(grpc): decompose grpc_service.rs into submodules ([#988](https://github.com/everruns/everruns/pull/988)) by [@chaliy](https://github.com/chaliy)
- refactor(storage): split repositories.rs and memory.rs god objects into per-entity modules ([#986](https://github.com/everruns/everruns/pull/986), [#987](https://github.com/everruns/everruns/pull/987)) by [@chaliy](https://github.com/chaliy)
- refactor(core): simplify ReasonAtom by removing 6 generic type parameters ([#983](https://github.com/everruns/everruns/pull/983)) by [@chaliy](https://github.com/chaliy)
- refactor(durable): replace Option<Option<T>> with UpdateField<T> enum ([#933](https://github.com/everruns/everruns/pull/933)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash 008-013 into 008_v0.8.7 ([#1030](https://github.com/everruns/everruns/pull/1030)) by [@chaliy](https://github.com/chaliy)
- chore(cli): bump everruns-sdk to v0.1.5 ([#1019](https://github.com/everruns/everruns/pull/1019)) by [@chaliy](https://github.com/chaliy)
- chore(config): add shared config crate, unify env-loading pattern ([#1007](https://github.com/everruns/everruns/pull/1007)) by [@chaliy](https://github.com/chaliy)
- chore(ship): add structured security review and enforce review comment resolution ([#1015](https://github.com/everruns/everruns/pull/1015)) by [@chaliy](https://github.com/chaliy)
- chore(skills): ship skill should analyze non-blocking review comments ([#1012](https://github.com/everruns/everruns/pull/1012)) by [@chaliy](https://github.com/chaliy)
- chore(core): audit and clean up #[allow(dead_code)] annotations ([#972](https://github.com/everruns/everruns/pull/972)) by [@chaliy](https://github.com/chaliy)
- chore(core): bump bashkit to v0.1.11 ([#940](https://github.com/everruns/everruns/pull/940)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): review stale in-progress linear issues ([#928](https://github.com/everruns/everruns/pull/928)) by [@chaliy](https://github.com/chaliy)
- chore(shipping): require final review sweep before merge ([#924](https://github.com/everruns/everruns/pull/924)) by [@chaliy](https://github.com/chaliy)
- chore(specs): enforce mandatory smoke testing in shipping requirements ([#903](https://github.com/everruns/everruns/pull/903)) by [@chaliy](https://github.com/chaliy)
- chore(skills): enforce /ship delegation in process-issues skill ([#904](https://github.com/everruns/everruns/pull/904)) by [@chaliy](https://github.com/chaliy)
- chore: convert process-issues command to goal-oriented skill ([#895](https://github.com/everruns/everruns/pull/895)) by [@chaliy](https://github.com/chaliy)
- chore: co-locate integration specs with their crates ([#896](https://github.com/everruns/everruns/pull/896)) by [@chaliy](https://github.com/chaliy)
- chore: add technical debt analysis to maintenance skill ([#891](https://github.com/everruns/everruns/pull/891)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): add GitHub security checks to maintenance requirements ([#898](https://github.com/everruns/everruns/pull/898)) by [@chaliy](https://github.com/chaliy)
- chore(ui): upgrade Next.js from 16.1.7 to 16.2.0 ([#977](https://github.com/everruns/everruns/pull/977)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump h3 from 1.15.6 to 1.15.9 ([#975](https://github.com/everruns/everruns/pull/975))
- chore(deps): bump rustls-webpki from 0.103.9 to 0.103.10 ([#976](https://github.com/everruns/everruns/pull/976))
- docs: extend all short meta descriptions to 150+ chars for SEO ([#900](https://github.com/everruns/everruns/pull/900)) by [@chaliy](https://github.com/chaliy)
- test(daytona): add UI test case for Daytona OpenUI connection flow ([#916](https://github.com/everruns/everruns/pull/916)) by [@chaliy](https://github.com/chaliy)
- test(ui): add global chat test cases for agent creation and execution ([#901](https://github.com/everruns/everruns/pull/901)) by [@chaliy](https://github.com/chaliy)
- test: add agent, session, and org creation test cases ([#892](https://github.com/everruns/everruns/pull/892)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.6 → 0.8.7:** Migrations have been squashed. Requires a fresh database if upgrading from pre-0.8.6.

## [0.8.6] - 2026-03-15

### Highlights

- **Multitenancy & Org Scoping** — Models, providers, capabilities, harnesses, and derived capabilities are now properly scoped to the owning organization with ownership validation on create ([#845](https://github.com/everruns/everruns/pull/845), [#850](https://github.com/everruns/everruns/pull/850), [#851](https://github.com/everruns/everruns/pull/851), [#852](https://github.com/everruns/everruns/pull/852))
- **Permissions Groundwork** — New permission resolver contract wired into AuthState and config endpoints, laying the foundation for fine-grained access control ([#836](https://github.com/everruns/everruns/pull/836), [#862](https://github.com/everruns/everruns/pull/862))
- **Durable Engine Improvements** — Pre-load count check, snapshot path limit, and continue-as-new for long-running workflows; partial output preserved on stream errors ([#839](https://github.com/everruns/everruns/pull/839), [#877](https://github.com/everruns/everruns/pull/877))
- **UI Polish** — Archive/delete entity states, filter dropdowns, model install/uninstall, org setup page, inline connection setup, tools list in LLM details ([#843](https://github.com/everruns/everruns/pull/843), [#814](https://github.com/everruns/everruns/pull/814), [#855](https://github.com/everruns/everruns/pull/855), [#865](https://github.com/everruns/everruns/pull/865))
- **Localization** — Started backend locale propagation support ([#830](https://github.com/everruns/everruns/pull/830))

### What's Changed

- feat(core): add permission resolver contract ([#836](https://github.com/everruns/everruns/pull/836)) by [@chaliy](https://github.com/chaliy)
- feat(server): wire PermissionResolver into AuthState and config endpoints ([#862](https://github.com/everruns/everruns/pull/862)) by [@chaliy](https://github.com/chaliy)
- feat(durable): pre-load count check, snapshot path limit, continue-as-new ([#839](https://github.com/everruns/everruns/pull/839)) by [@chaliy](https://github.com/chaliy)
- feat(session): add backend locale propagation ([#830](https://github.com/everruns/everruns/pull/830)) by [@chaliy](https://github.com/chaliy)
- feat(session): add initial files for agents and harnesses ([#832](https://github.com/everruns/everruns/pull/832)) by [@chaliy](https://github.com/chaliy)
- feat(connections): inline connection setup via client-side tool call ([#814](https://github.com/everruns/everruns/pull/814)) by [@chaliy](https://github.com/chaliy)
- feat(lifecycle): add archive and delete entity states ([#843](https://github.com/everruns/everruns/pull/843)) by [@chaliy](https://github.com/chaliy)
- feat(sdk): update SDK to v0.1.4 and add agents list pagination ([#846](https://github.com/everruns/everruns/pull/846)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add model install/uninstall toggle and org default model selector ([#868](https://github.com/everruns/everruns/pull/868)) by [@chaliy](https://github.com/chaliy)
- feat(ui): replace archive checkboxes with filter dropdown on all list pages ([#855](https://github.com/everruns/everruns/pull/855)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add tools list and copy button to LLM generation details ([#858](https://github.com/everruns/everruns/pull/858)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add org setup page after creation ([#865](https://github.com/everruns/everruns/pull/865)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate provider ownership for model create ([#845](https://github.com/everruns/everruns/pull/845)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate harness and model ownership on session create ([#850](https://github.com/everruns/everruns/pull/850)) by [@chaliy](https://github.com/chaliy)
- fix(api): scope agent and harness default model ids ([#844](https://github.com/everruns/everruns/pull/844)) by [@chaliy](https://github.com/chaliy)
- fix(api): query DB for org membership instead of stale auth context ([#857](https://github.com/everruns/everruns/pull/857)) by [@chaliy](https://github.com/chaliy)
- fix(api): bind session schedule routes to parent session ([#848](https://github.com/everruns/everruns/pull/848)) by [@chaliy](https://github.com/chaliy)
- fix(storage): scope llm model provider joins to org ([#851](https://github.com/everruns/everruns/pull/851)) by [@chaliy](https://github.com/chaliy)
- fix(session): scope derived capabilities to org-owned refs ([#852](https://github.com/everruns/everruns/pull/852)) by [@chaliy](https://github.com/chaliy)
- fix(org): add default and base harness settings ([#849](https://github.com/everruns/everruns/pull/849)) by [@chaliy](https://github.com/chaliy)
- fix(auth): query DB for org memberships in /v1/auth/me (none mode) ([#863](https://github.com/everruns/everruns/pull/863)) by [@chaliy](https://github.com/chaliy)
- fix(auth): grant admin users owner role in default org ([#873](https://github.com/everruns/everruns/pull/873)) by [@chaliy](https://github.com/chaliy)
- fix(worker): record circuit breaker failure on LLM errors ([#853](https://github.com/everruns/everruns/pull/853)) by [@chaliy](https://github.com/chaliy)
- fix(worker): prevent duplicate error events on transient LLM failures ([#869](https://github.com/everruns/everruns/pull/869)) by [@chaliy](https://github.com/chaliy)
- fix(worker): add connection_required handling to durable worker ([#871](https://github.com/everruns/everruns/pull/871)) by [@chaliy](https://github.com/chaliy)
- fix(core): revert tool_search auto-enable, keep capability-driven ([#860](https://github.com/everruns/everruns/pull/860)) by [@chaliy](https://github.com/chaliy)
- fix(core): auto-enable tool_search for GPT-5.4 and remove Daytona prompt duplication ([#859](https://github.com/everruns/everruns/pull/859)) by [@chaliy](https://github.com/chaliy)
- fix(reason): preserve partial output on trailing stream errors ([#877](https://github.com/everruns/everruns/pull/877)) by [@chaliy](https://github.com/chaliy)
- fix(protocol): include missing fields in proto session conversion ([#875](https://github.com/everruns/everruns/pull/875)) by [@chaliy](https://github.com/chaliy)
- fix(config): enforce real user git identity via SessionStart hook ([#861](https://github.com/everruns/everruns/pull/861)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): block internal network targets ([#838](https://github.com/everruns/everruns/pull/838)) by [@chaliy](https://github.com/chaliy)
- fix(openui): add error boundary around Renderer for malformed ElementNode objects ([#835](https://github.com/everruns/everruns/pull/835)) by [@chaliy](https://github.com/chaliy)
- fix(ui-security): fail closed on auth bootstrap errors ([#840](https://github.com/everruns/everruns/pull/840)) by [@chaliy](https://github.com/chaliy)
- fix(ui): self-host Caveat font to avoid Google Fonts CSP drift ([#847](https://github.com/everruns/everruns/pull/847)) by [@chaliy](https://github.com/chaliy)
- fix(ui): always show session title edit button ([#856](https://github.com/everruns/everruns/pull/856)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate single-row tool activity timeline display ([#864](https://github.com/everruns/everruns/pull/864)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redirect to entity list page on org switch ([#866](https://github.com/everruns/everruns/pull/866)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify archive filter label to "Show archived" ([#870](https://github.com/everruns/everruns/pull/870)) by [@chaliy](https://github.com/chaliy)
- fix(ui): match filter button size with sibling buttons ([#867](https://github.com/everruns/everruns/pull/867)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent horizontal scroll on schedules page ([#854](https://github.com/everruns/everruns/pull/854)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove chat composer divider ([#842](https://github.com/everruns/everruns/pull/842)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove always-visible scroll buttons from select dropdowns ([#878](https://github.com/everruns/everruns/pull/878)) by [@chaliy](https://github.com/chaliy)
- fix(ui): improve connection-required banner readability ([#880](https://github.com/everruns/everruns/pull/880)) by [@chaliy](https://github.com/chaliy)
- refactor(core): remove system prompt duplication with tool definitions ([#879](https://github.com/everruns/everruns/pull/879)) by [@chaliy](https://github.com/chaliy)
- refactor(core): remove stream-level retry from ReasonAtom, classify LLM errors ([#872](https://github.com/everruns/everruns/pull/872)) by [@chaliy](https://github.com/chaliy)
- chore(core): bump bashkit v0.1.8 → v0.1.10 ([#876](https://github.com/everruns/everruns/pull/876)) by [@chaliy](https://github.com/chaliy)
- chore(server): squash post-0.8.5 migrations into single 0.8.6 migration ([#874](https://github.com/everruns/everruns/pull/874)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): add invokable maintenance skill ([#833](https://github.com/everruns/everruns/pull/833)) by [@chaliy](https://github.com/chaliy)
- chore(ship): move ship workflow into invokable skill ([#834](https://github.com/everruns/everruns/pull/834)) by [@chaliy](https://github.com/chaliy)
- chore(agents): require latest remote main in worktrees ([#837](https://github.com/everruns/everruns/pull/837)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.5 → 0.8.6:** Requires fresh database. Run migrations with `just migrate` or start with `just start-all`.

## [0.8.5] - 2026-03-12

### Highlights

- **Browserless Integration** — Browser automation for agents via Browserless ([#776](https://github.com/everruns/everruns/pull/776))
- **Slack Thread Context** — Bot receives full thread context when first mentioned mid-thread ([#768](https://github.com/everruns/everruns/pull/768))
- **Preview of OpenUI Generative UI** — Dynamic generative UI capability for agents ([#790](https://github.com/everruns/everruns/pull/790))
- **Global Search & Command Palette** — Cmd+K to search sessions, navigate, and run commands ([#767](https://github.com/everruns/everruns/pull/767))
- **Performance Improvements** — GIN-indexed tsvector event search, durable snapshot checkpointing, paginated event loading ([#787](https://github.com/everruns/everruns/pull/787), [#794](https://github.com/everruns/everruns/pull/794))

### What's Changed

- feat(durable): add snapshot checkpointing for workflow event replay ([#794](https://github.com/everruns/everruns/pull/794)) by [@chaliy](https://github.com/chaliy)
- feat(openui): implement OpenUI generative UI capability ([#790](https://github.com/everruns/everruns/pull/790)) by [@chaliy](https://github.com/chaliy)
- feat: paginated event loading for large sessions (EVE-82, EVE-83) by [@chaliy](https://github.com/chaliy)
- feat(ui): bottom-anchored chat scroll with new messages indicator ([#781](https://github.com/everruns/everruns/pull/781)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add exponential backoff to SSE reconnection ([#779](https://github.com/everruns/everruns/pull/779)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add Browserless browser automation integration ([#776](https://github.com/everruns/everruns/pull/776)) by [@chaliy](https://github.com/chaliy)
- feat(search): global search & command palette (Cmd+K) ([#767](https://github.com/everruns/everruns/pull/767)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): add ownership metadata labels to sandbox creation ([#772](https://github.com/everruns/everruns/pull/772)) by [@chaliy](https://github.com/chaliy)
- feat(slack): inject thread context when bot is first mentioned mid-thread ([#768](https://github.com/everruns/everruns/pull/768)) by [@chaliy](https://github.com/chaliy)
- feat(core): set GPT-5.4 as default model ([#762](https://github.com/everruns/everruns/pull/762)) by [@chaliy](https://github.com/chaliy)
- feat(connections): add generic API key verification for connected accounts ([#760](https://github.com/everruns/everruns/pull/760)) by [@chaliy](https://github.com/chaliy)
- feat(ui): move MCP Servers from Settings to Building Blocks ([#761](https://github.com/everruns/everruns/pull/761)) by [@chaliy](https://github.com/chaliy)
- feat(core): add execution phases and iteration tracking ([#759](https://github.com/everruns/everruns/pull/759)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add durable queues management page ([#754](https://github.com/everruns/everruns/pull/754)) by [@chaliy](https://github.com/chaliy)
- feat(ui): collapse durable execution sidebar by default ([#756](https://github.com/everruns/everruns/pull/756)) by [@chaliy](https://github.com/chaliy)
- fix(server): log migration errors through tracing before propagating ([#800](https://github.com/everruns/everruns/pull/800)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix duplicate React key in command palette navigation ([#798](https://github.com/everruns/everruns/pull/798)) by [@chaliy](https://github.com/chaliy)
- fix(storage): clear existing default model before setting new one ([#797](https://github.com/everruns/everruns/pull/797)) by [@chaliy](https://github.com/chaliy)
- fix(ui): close global search on ESC and route navigation ([#795](https://github.com/everruns/everruns/pull/795)) by [@chaliy](https://github.com/chaliy)
- fix(storage): replace ILIKE event search with GIN-indexed tsvector ([#787](https://github.com/everruns/everruns/pull/787)) by [@chaliy](https://github.com/chaliy)
- fix(durable): use SELECT COUNT(*) for event counting ([#782](https://github.com/everruns/everruns/pull/782)) by [@chaliy](https://github.com/chaliy)
- fix(slack): ensure long_description meets Slack's 174-char minimum ([#780](https://github.com/everruns/everruns/pull/780)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show completed turn duration in chat ([#778](https://github.com/everruns/everruns/pull/778)) by [@chaliy](https://github.com/chaliy)
- fix(docs): upgrade Astro docs site to v6 ([#777](https://github.com/everruns/everruns/pull/777)) by [@chaliy](https://github.com/chaliy)
- fix(slack): expose external_actor in API Message response ([#771](https://github.com/everruns/everruns/pull/771)) by [@chaliy](https://github.com/chaliy)
- fix(apps): auto-complete Slack setup checklist steps 4 and 5 ([#770](https://github.com/everruns/everruns/pull/770)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use git clone via exec instead of broken /git/clone endpoint ([#766](https://github.com/everruns/everruns/pull/766)) by [@chaliy](https://github.com/chaliy)
- fix(worker): wire all stores into durable act_activity ([#763](https://github.com/everruns/everruns/pull/763)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use /home/daytona as workspace path ([#765](https://github.com/everruns/everruns/pull/765)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): remove API key prerequisite from Coder agent prompt ([#757](https://github.com/everruns/everruns/pull/757)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify inline tool transcript ([#758](https://github.com/everruns/everruns/pull/758)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): return 409 instead of 500 on duplicate file creation ([#755](https://github.com/everruns/everruns/pull/755)) by [@chaliy](https://github.com/chaliy)
- fix(server): increase session files upload body limit to 10MB ([#751](https://github.com/everruns/everruns/pull/751)) by [@chaliy](https://github.com/chaliy)
- fix(ui): render folder action icons inline with folder name ([#752](https://github.com/everruns/everruns/pull/752)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix code block rendering in chat messages ([#753](https://github.com/everruns/everruns/pull/753)) by [@chaliy](https://github.com/chaliy)
- fix(vfs): correct folder detection in stat() ([#749](https://github.com/everruns/everruns/pull/749)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent workspace file tree from overflowing viewport ([#750](https://github.com/everruns/everruns/pull/750)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump fetchkit from 0.1.2 to 0.1.3 ([#802](https://github.com/everruns/everruns/pull/802))
- chore(migrations): squash post-0.8.4 migrations into 006_v0.8.5 ([#792](https://github.com/everruns/everruns/pull/792)) by [@chaliy](https://github.com/chaliy)
- test(slack): enforce credentials, add Slack integration tests to CI ([#789](https://github.com/everruns/everruns/pull/789)) by [@chaliy](https://github.com/chaliy)
- test(daytona): add live API integration tests ([#774](https://github.com/everruns/everruns/pull/774)) by [@chaliy](https://github.com/chaliy)

## [0.8.4] - 2026-03-08

### Highlights

- **Brave Search** — New connection provider and seed agent for Brave Search web search ([#716](https://github.com/everruns/everruns/pull/716))
- **Slack Bot** — Event-driven delivery dispatcher, file/legacy attachment support, per-app manifest generation ([#696](https://github.com/everruns/everruns/pull/696), [#717](https://github.com/everruns/everruns/pull/717), [#689](https://github.com/everruns/everruns/pull/689))
- **Performance Caching** — In-memory caches for encryption keys, model resolution, auth validation, skills, and agent capabilities ([#700](https://github.com/everruns/everruns/pull/700), [#701](https://github.com/everruns/everruns/pull/701), [#702](https://github.com/everruns/everruns/pull/702), [#705](https://github.com/everruns/everruns/pull/705), [#706](https://github.com/everruns/everruns/pull/706))
- **Valkey Rate Limiting** — Distributed rate limiting via Valkey replaces in-process limiters ([#690](https://github.com/everruns/everruns/pull/690))
- **Tool Search** — OpenAI GPT 5.4 tool_search capability for deferred tool loading ([#687](https://github.com/everruns/everruns/pull/687))

### What's Changed

- feat(brave-search): add connection provider, seed agent, and Doppler CI ([#716](https://github.com/everruns/everruns/pull/716)) by [@chaliy](https://github.com/chaliy)
- feat(slack): support file and legacy attachments in messages ([#717](https://github.com/everruns/everruns/pull/717)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unify chat and shell slate styling ([#718](https://github.com/everruns/everruns/pull/718)) by [@chaliy](https://github.com/chaliy)
- feat(ui): show ngrok instructions when Slack webhook URL is localhost ([#714](https://github.com/everruns/everruns/pull/714)) by [@chaliy](https://github.com/chaliy)
- feat(ui): show display names instead of raw IDs in all select dropdowns ([#713](https://github.com/everruns/everruns/pull/713)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add title and experimental badge to Chat page ([#711](https://github.com/everruns/everruns/pull/711)) by [@chaliy](https://github.com/chaliy)
- feat(ui): pluggable logout and createOrganization via AuthContext ([#709](https://github.com/everruns/everruns/pull/709)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add sidebar navigation registry with extension points ([#707](https://github.com/everruns/everruns/pull/707)) by [@chaliy](https://github.com/chaliy)
- feat(build): add optional sccache integration with S3 backend ([#704](https://github.com/everruns/everruns/pull/704)) by [@chaliy](https://github.com/chaliy)
- feat(slack): event-driven delivery dispatcher replaces 120s polling ([#696](https://github.com/everruns/everruns/pull/696)) by [@chaliy](https://github.com/chaliy)
- feat(ci): add sccache S3 backend for shared Rust compilation cache ([#693](https://github.com/everruns/everruns/pull/693)) by [@chaliy](https://github.com/chaliy)
- feat(durable): add generic queue semantics for standalone tasks ([#691](https://github.com/everruns/everruns/pull/691)) by [@chaliy](https://github.com/chaliy)
- feat(server): add Valkey for distributed rate limiting ([#690](https://github.com/everruns/everruns/pull/690)) by [@chaliy](https://github.com/chaliy)
- feat(slack): per-app manifest generation and setup guide ([#689](https://github.com/everruns/everruns/pull/689)) by [@chaliy](https://github.com/chaliy)
- feat(core): implement OpenAI tool_search capability for deferred tool loading ([#687](https://github.com/everruns/everruns/pull/687)) by [@chaliy](https://github.com/chaliy)
- feat(core): add ExternalActor for channel-agnostic user identity ([#688](https://github.com/everruns/everruns/pull/688)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add experimental badge for Chat and Apps ([#684](https://github.com/everruns/everruns/pull/684)) by [@chaliy](https://github.com/chaliy)
- feat(core): add apps feature flag ([#685](https://github.com/everruns/everruns/pull/685)) by [@chaliy](https://github.com/chaliy)
- feat(ui): redesign app creation flow with non-modal Slack config ([#683](https://github.com/everruns/everruns/pull/683)) by [@chaliy](https://github.com/chaliy)
- fix(dev): restore stop-all cleanup and caddy validation ([#728](https://github.com/everruns/everruns/pull/728)) by [@chaliy](https://github.com/chaliy)
- fix(multitenancy): remove DEFAULT_ORG_ID fallbacks from worker runtime paths ([#727](https://github.com/everruns/everruns/pull/727)) by [@chaliy](https://github.com/chaliy)
- fix(dev-parity): implement grep_files in DirectWorkerAdapters by [@chaliy](https://github.com/chaliy)
- fix(dev): isolate worktree port layout ([#726](https://github.com/everruns/everruns/pull/726)) by [@chaliy](https://github.com/chaliy)
- fix(model-sync): use decrypted provider keys and sync across all orgs by [@chaliy](https://github.com/chaliy)
- fix(mcp): pass decrypted API keys to MCP tool execution by [@chaliy](https://github.com/chaliy)
- fix(auth): validate GitHub App installation callback state by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify bash tool result details ([#725](https://github.com/everruns/everruns/pull/725)) by [@chaliy](https://github.com/chaliy)
- fix(ui): move Event Subscriptions card inside grid layout ([#715](https://github.com/everruns/everruns/pull/715)) by [@chaliy](https://github.com/chaliy)
- fix(slack): add missing users:read scope to bot manifest ([#712](https://github.com/everruns/everruns/pull/712)) by [@chaliy](https://github.com/chaliy)
- fix(ui): stop experimental badge overlapping chat content ([#699](https://github.com/everruns/everruns/pull/699)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): prevent init-cloud-env hangs on downloads ([#698](https://github.com/everruns/everruns/pull/698)) by [@chaliy](https://github.com/chaliy)
- fix(core): move tool_search guard to RuntimeAgentBuilder ([#703](https://github.com/everruns/everruns/pull/703)) by [@chaliy](https://github.com/chaliy)
- fix(slack): correct answer mapping, dedup events, and stream progress ([#686](https://github.com/everruns/everruns/pull/686)) by [@chaliy](https://github.com/chaliy)
- fix(docs): correct Slack OAuth scope from app_mentions:events to app_mentions:read ([#682](https://github.com/everruns/everruns/pull/682)) by [@chaliy](https://github.com/chaliy)
- fix: deduplicate moka workspace dep + add sequential merge spec ([#708](https://github.com/everruns/everruns/pull/708)) by [@chaliy](https://github.com/chaliy)
- perf(ci): build all Rust Docker images in single builder stage ([#710](https://github.com/everruns/everruns/pull/710)) by [@chaliy](https://github.com/chaliy)
- perf(encryption): cache decrypted encryption keys in memory ([#706](https://github.com/everruns/everruns/pull/706)) by [@chaliy](https://github.com/chaliy)
- perf(server): deduplicate get_agent_capabilities() calls ([#705](https://github.com/everruns/everruns/pull/705)) by [@chaliy](https://github.com/chaliy)
- perf(skills): cache active skills list per org with 5-min TTL ([#702](https://github.com/everruns/everruns/pull/702)) by [@chaliy](https://github.com/chaliy)
- perf(llm): cache model/provider resolution with 1-hour TTL ([#701](https://github.com/everruns/everruns/pull/701)) by [@chaliy](https://github.com/chaliy)
- perf(auth): cache API key auth validation with 5-min TTL ([#700](https://github.com/everruns/everruns/pull/700)) by [@chaliy](https://github.com/chaliy)
- refactor(server): centralize runtime credential and grep resolution paths ([#731](https://github.com/everruns/everruns/pull/731)) by [@chaliy](https://github.com/chaliy)
- refactor: remove CodeSandbox integration ([#719](https://github.com/everruns/everruns/pull/719), [#720](https://github.com/everruns/everruns/pull/720)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): standardize page headers to inline pattern ([#694](https://github.com/everruns/everruns/pull/694)) by [@chaliy](https://github.com/chaliy)
- test(runtime): add adapter contract coverage and security-focused negative tests ([#730](https://github.com/everruns/everruns/pull/730)) by [@chaliy](https://github.com/chaliy)
- chore(server): squash v0.8.4 migration into 005_v0.8.4.sql ([#733](https://github.com/everruns/everruns/pull/733)) by [@chaliy](https://github.com/chaliy)
- chore(dev): remove Jaeger UI from local dev and example ([#729](https://github.com/everruns/everruns/pull/729)) by [@chaliy](https://github.com/chaliy)
- chore: reorganize Linear tickets to OSS project ([#695](https://github.com/everruns/everruns/pull/695)) by [@chaliy](https://github.com/chaliy)
- refactor(multitenancy): thread org_id through worker and image-resolution interfaces ([#732](https://github.com/everruns/everruns/pull/732)) by [@chaliy](https://github.com/chaliy)
- ci: optimize pipeline — pre-build test binary, 8-core runner, combine test invocations ([#697](https://github.com/everruns/everruns/pull/697)) by [@chaliy](https://github.com/chaliy)

## [0.8.3] - 2026-03-06

### Highlights

- **GPT-5.4 Support** — Full model profiles for GPT-5.4 and GPT-5.4 Pro with input token limits ([#653](https://github.com/everruns/everruns/pull/653), [#654](https://github.com/everruns/everruns/pull/654), [#657](https://github.com/everruns/everruns/pull/657))
- **Custom Commands** — Slash command system with UI autocomplete ([#667](https://github.com/everruns/everruns/pull/667))
- **Security Hardening** — Per-IP rate limiting, structured audit logging, mTLS, security headers, account enumeration prevention ([#627](https://github.com/everruns/everruns/pull/627), [#633](https://github.com/everruns/everruns/pull/633), [#634](https://github.com/everruns/everruns/pull/634), [#636](https://github.com/everruns/everruns/pull/636), [#641](https://github.com/everruns/everruns/pull/641))
- **Durable Engine Scaling** — Multi-instance control plane, capacity-aware fair-share claiming, worker backpressure ([#637](https://github.com/everruns/everruns/pull/637), [#638](https://github.com/everruns/everruns/pull/638), [#639](https://github.com/everruns/everruns/pull/639), [#640](https://github.com/everruns/everruns/pull/640))
- **DuckDuckGo Search** — DuckDuckGo Instant Answer search integration ([#663](https://github.com/everruns/everruns/pull/663))

### What's Changed

- feat(core): add GPT-5.4 and GPT-5.4 Pro model profiles ([#653](https://github.com/everruns/everruns/pull/653)) by [@chaliy](https://github.com/chaliy)
- feat(core): add GPT-5.4 model profiles and integration tests ([#654](https://github.com/everruns/everruns/pull/654)) by [@chaliy](https://github.com/chaliy)
- feat(core): add optional input token limit to LlmModelLimits ([#657](https://github.com/everruns/everruns/pull/657)) by [@chaliy](https://github.com/chaliy)
- feat(commands): add custom commands system with UI autocomplete ([#667](https://github.com/everruns/everruns/pull/667)) by [@chaliy](https://github.com/chaliy)
- feat(duckduckgo): add DuckDuckGo Instant Answer search integration ([#663](https://github.com/everruns/everruns/pull/663)) by [@chaliy](https://github.com/chaliy)
- feat(users): add profile page with full name editing ([#649](https://github.com/everruns/everruns/pull/649)) by [@chaliy](https://github.com/chaliy)
- feat(ui): Claude Code-style bash tool rendering ([#644](https://github.com/everruns/everruns/pull/644)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add list_capabilities tool to platform management ([#642](https://github.com/everruns/everruns/pull/642)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add risk level classification and admin approval ([#631](https://github.com/everruns/everruns/pull/631)) by [@chaliy](https://github.com/chaliy)
- feat(grpc): add mutual TLS (mTLS) support for worker-server communication ([#641](https://github.com/everruns/everruns/pull/641)) by [@chaliy](https://github.com/chaliy)
- feat(grpc): add gRPC support for sqldb_store in WorkerAdapters ([#645](https://github.com/everruns/everruns/pull/645)) by [@chaliy](https://github.com/chaliy)
- feat(durable): resource-based worker backpressure ([#638](https://github.com/everruns/everruns/pull/638)) by [@chaliy](https://github.com/chaliy)
- feat(durable): capacity-aware fair-share task claiming ([#639](https://github.com/everruns/everruns/pull/639)) by [@chaliy](https://github.com/chaliy)
- feat(durable): load-proportional claim jitter ([#640](https://github.com/everruns/everruns/pull/640)) by [@chaliy](https://github.com/chaliy)
- feat(server): multi-instance control plane support ([#637](https://github.com/everruns/everruns/pull/637)) by [@chaliy](https://github.com/chaliy)
- feat(server): structured audit logging for auth events ([#636](https://github.com/everruns/everruns/pull/636)) by [@chaliy](https://github.com/chaliy)
- feat(server): add security response headers ([#634](https://github.com/everruns/everruns/pull/634)) by [@chaliy](https://github.com/chaliy)
- feat(auth): add per-IP rate limiting on auth endpoints ([#627](https://github.com/everruns/everruns/pull/627)) by [@chaliy](https://github.com/chaliy)
- feat(storage): add encrypted system_prompt columns ([#630](https://github.com/everruns/everruns/pull/630)) by [@chaliy](https://github.com/chaliy)
- feat(chat): add run-agent, harness-avoidance, and confirmation guidelines to chat system prompt ([#648](https://github.com/everruns/everruns/pull/648)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add horizontal navigation tabs ([#662](https://github.com/everruns/everruns/pull/662)) by [@chaliy](https://github.com/chaliy)
- feat: Slack bot integration with Apps abstraction ([#671](https://github.com/everruns/everruns/pull/671)) by [@chaliy](https://github.com/chaliy)
- fix(ui): provider icons invisible on light theme ([#670](https://github.com/everruns/everruns/pull/670)) by [@chaliy](https://github.com/chaliy)
- fix(ui): render plain URLs as links in chat markdown ([#646](https://github.com/everruns/everruns/pull/646)) by [@chaliy](https://github.com/chaliy)
- fix(ui): default to Generic harness in New Session dialog ([#632](https://github.com/everruns/everruns/pull/632)) by [@chaliy](https://github.com/chaliy)
- fix(vfs): block deletion of readonly files ([#669](https://github.com/everruns/everruns/pull/669)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): scope platform store to session org and fix public URL default ([#647](https://github.com/everruns/everruns/pull/647)) by [@chaliy](https://github.com/chaliy)
- fix(auth): prevent account enumeration via registration endpoint ([#633](https://github.com/everruns/everruns/pull/633)) by [@chaliy](https://github.com/chaliy)
- fix(api): add regex pattern length limit on grep endpoint ([#629](https://github.com/everruns/everruns/pull/629)) by [@chaliy](https://github.com/chaliy)
- fix(server): warn when DATABASE_URL lacks TLS in production ([#628](https://github.com/everruns/everruns/pull/628)) by [@chaliy](https://github.com/chaliy)
- fix(worker): enforce WorkerAdapters parity at compile time ([#643](https://github.com/everruns/everruns/pull/643)) by [@chaliy](https://github.com/chaliy)
- fix(docker): pin UI builder stage to amd64 to avoid QEMU SIGILL ([#652](https://github.com/everruns/everruns/pull/652)) by [@chaliy](https://github.com/chaliy)
- fix(ci): merge env-var SSE tests to prevent flaky race condition ([#656](https://github.com/everruns/everruns/pull/656)) by [@chaliy](https://github.com/chaliy)
- fix(ci): skip arm64 QEMU build for UI Docker image ([#651](https://github.com/everruns/everruns/pull/651)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): adjust risk levels, rename capabilities, add Daytona docs ([#675](https://github.com/everruns/everruns/pull/675)) by [@chaliy](https://github.com/chaliy)
- refactor: rename GRPC_* env vars to WORKER_GRPC_* prefix ([#635](https://github.com/everruns/everruns/pull/635)) by [@chaliy](https://github.com/chaliy)
- revert(server): remove system_prompt_encrypted ([#659](https://github.com/everruns/everruns/pull/659)) by [@chaliy](https://github.com/chaliy)
- docs(capabilities): add Capabilities navigation tab with top 15 capability reference pages ([#672](https://github.com/everruns/everruns/pull/672)) by [@chaliy](https://github.com/chaliy)
- docs: add tutorial for building agents using the Everruns SDK ([#661](https://github.com/everruns/everruns/pull/661)) by [@chaliy](https://github.com/chaliy)
- docs: reduce duplication in building-agents-using-sdk tutorial ([#674](https://github.com/everruns/everruns/pull/674)) by [@chaliy](https://github.com/chaliy)
- docs: improve meta descriptions for SEO ([#660](https://github.com/everruns/everruns/pull/660)) by [@chaliy](https://github.com/chaliy)
- chore: pre-release maintenance — update dependencies ([#668](https://github.com/everruns/everruns/pull/668)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash 005_apps into 004_v0.8.3 ([#678](https://github.com/everruns/everruns/pull/678)) by [@chaliy](https://github.com/chaliy)
- chore(db): squash migrations 004-006 into 004_v0.8.3 ([#673](https://github.com/everruns/everruns/pull/673)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump dompurify from 3.3.1 to 3.3.2 in /apps/docs ([#655](https://github.com/everruns/everruns/pull/655)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump svgo from 4.0.0 to 4.0.1 in /apps/docs ([#650](https://github.com/everruns/everruns/pull/650)) by [@dependabot](https://github.com/dependabot)
- chore(docs): add IndexNow verification key file ([#658](https://github.com/everruns/everruns/pull/658)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.2 → 0.8.3:** Requires fresh database (new migration squash). The `GRPC_*` environment variables have been renamed to `WORKER_GRPC_*` — update your configuration accordingly.

## [0.8.2] - 2026-03-01

### Highlights

- **Global Chat Page** — New global chat page and Chat harness for direct agent conversations ([#602](https://github.com/everruns/everruns/pull/602), [#608](https://github.com/everruns/everruns/pull/608))
- **Platform Management Capability** — New capability for platform operations, wired through Chat harness and gRPC workers ([#587](https://github.com/everruns/everruns/pull/587), [#608](https://github.com/everruns/everruns/pull/608), [#615](https://github.com/everruns/everruns/pull/615), [#622](https://github.com/everruns/everruns/pull/622))
- **SSE Reliability** — Periodic heartbeat comments, HTTP/2 flow control tuning, 1000-connection limit, reliable event ordering via sequence resolution ([#604](https://github.com/everruns/everruns/pull/604), [#584](https://github.com/everruns/everruns/pull/584), [#585](https://github.com/everruns/everruns/pull/585), [#597](https://github.com/everruns/everruns/pull/597), [#606](https://github.com/everruns/everruns/pull/606))
- **Durable Dashboard Metrics** — Time-series graphs, accurate worker counts, throughput rates, and dev-mode support ([#578](https://github.com/everruns/everruns/pull/578), [#590](https://github.com/everruns/everruns/pull/590), [#596](https://github.com/everruns/everruns/pull/596), [#610](https://github.com/everruns/everruns/pull/610))
- **OpenAI Context Caching** — Thread `previous_response_id` for server-side context caching across turns ([#594](https://github.com/everruns/everruns/pull/594))

### What's Changed

- fix(worker): wire platform_store in DurableWorker act_activity path ([#622](https://github.com/everruns/everruns/pull/622))
- chore: add /process-issues command for Linear issue processing ([#619](https://github.com/everruns/everruns/pull/619))
- chore(specs): security audit — 12 findings with threat model updates ([#618](https://github.com/everruns/everruns/pull/618))
- feat(ship): add code simplification and security review phases ([#616](https://github.com/everruns/everruns/pull/616))
- chore(ship): add impact awareness to Phase 5 quality gates ([#617](https://github.com/everruns/everruns/pull/617))
- fix(server): downgrade missing directory log from error to debug ([#614](https://github.com/everruns/everruns/pull/614))
- fix(worker): implement PlatformStore for gRPC workers ([#615](https://github.com/everruns/everruns/pull/615))
- chore(deps): upgrade bashkit v0.1.7 → v0.1.8 ([#613](https://github.com/everruns/everruns/pull/613))
- fix(example): always pull fresh images on start ([#612](https://github.com/everruns/everruns/pull/612))
- chore(deps): upgrade everruns-sdk v0.1.2 → v0.1.3 ([#611](https://github.com/everruns/everruns/pull/611))
- fix(durable): fix dashboard metrics for dev mode and show all statuses ([#610](https://github.com/everruns/everruns/pull/610))
- feat(ui): add preview tabs to harness detail and edit pages ([#607](https://github.com/everruns/everruns/pull/607))
- refactor(bench): always use llmsim-latency model in load tests ([#609](https://github.com/everruns/everruns/pull/609))
- feat(capabilities): register platform_management in Chat harness, rename to Platform Chat ([#608](https://github.com/everruns/everruns/pull/608))
- fix(server): resolve since_id to sequence for reliable event ordering ([#606](https://github.com/everruns/everruns/pull/606))
- chore(deps): bump the npm_and_yarn group across 1 directory with 1 update ([#605](https://github.com/everruns/everruns/pull/605))
- feat(sse): add periodic heartbeat comments to all SSE streams ([#604](https://github.com/everruns/everruns/pull/604))
- feat(ui,server): add global chat page and Chat harness ([#602](https://github.com/everruns/everruns/pull/602))
- fix(server): eliminate env var race in Http2FlowConfig tests ([#601](https://github.com/everruns/everruns/pull/601))
- fix(load-test): remove SSE retry cap to use SDK default unlimited reconnects ([#599](https://github.com/everruns/everruns/pull/599))
- chore(deps): upgrade bashkit v0.1.6 → v0.1.7 ([#598](https://github.com/everruns/everruns/pull/598))
- fix(sse): bump SDK with SSE disconnect fix, configurable cycling ([#597](https://github.com/everruns/everruns/pull/597))
- fix(durable): show workflow/task throughput rates instead of gauges ([#596](https://github.com/everruns/everruns/pull/596))
- fix(docs): block /cdn-cgi/ in robots.txt ([#595](https://github.com/everruns/everruns/pull/595))
- feat(core): thread previous_response_id for OpenAI server-side context caching ([#594](https://github.com/everruns/everruns/pull/594))
- fix(ci): stop cancelling in-progress CI runs on main ([#593](https://github.com/everruns/everruns/pull/593))
- chore: upgrade Node.js from 20 to 22 LTS ([#592](https://github.com/everruns/everruns/pull/592))
- chore(ui): update oxfmt to 0.35.0 ([#591](https://github.com/everruns/everruns/pull/591))
- fix(durable): fix dashboard worker count and metrics accuracy ([#590](https://github.com/everruns/everruns/pull/590))
- fix(docs): resolve SEO crawling issues (308 redirects, excluded pages) ([#589](https://github.com/everruns/everruns/pull/589))
- fix(bench): eagerly connect SSE stream before sending messages ([#588](https://github.com/everruns/everruns/pull/588))
- feat(core): add platform management capability ([#587](https://github.com/everruns/everruns/pull/587))
- fix(server): configure HTTP/2 flow control for high-concurrency SSE ([#584](https://github.com/everruns/everruns/pull/584))
- chore(specs): remove code-derivable content, link to source files ([#586](https://github.com/everruns/everruns/pull/586))
- fix(server): bump per-org SSE connection limit from 50 to 1000 ([#585](https://github.com/everruns/everruns/pull/585))
- fix(bench): use SDK EventStream for SSE reconnection in load test ([#582](https://github.com/everruns/everruns/pull/582))
- feat(server): add types positive filter to events endpoints ([#581](https://github.com/everruns/everruns/pull/581))
- feat(durable): add metrics time-series graphs to overview dashboard ([#578](https://github.com/everruns/everruns/pull/578))
- feat(fake_aws): autonomous Cost & Security Auditor with rich seed data ([#577](https://github.com/everruns/everruns/pull/577))
- feat(bench): replace polling with SSE for turn completion detection ([#580](https://github.com/everruns/everruns/pull/580))
- fix(durable): add summary to workers list response ([#579](https://github.com/everruns/everruns/pull/579))
- fix(server): use axum_extra::extract::Query for SSE exclude param ([#575](https://github.com/everruns/everruns/pull/575))
- chore(ship): expand /ship command to enforce full quality workflow ([#576](https://github.com/everruns/everruns/pull/576))
- feat(llmsim): add latency and streaming simulation for benchmarks ([#574](https://github.com/everruns/everruns/pull/574))
- fix(ui): merge orphan input.message into turn.started in trajectory view ([#573](https://github.com/everruns/everruns/pull/573))
- refactor(server): extract ServerAppBuilder, remove server::run() ([#572](https://github.com/everruns/everruns/pull/572))
- fix(durable): skip postgres-dependent tests without PostgreSQL ([#571](https://github.com/everruns/everruns/pull/571))

## [0.8.1] - 2026-02-22

### Highlights

- **Load Testing Infrastructure** — New load testing framework with llmsim mock LLM server and durable execution race condition fix ([#568](https://github.com/everruns/everruns/pull/568))
- **Dashboard Stats** — Total sessions count and improved session stats accuracy ([#564](https://github.com/everruns/everruns/pull/564))
- **CI & Docs Improvements** — Docker publish fix for release tags, SEO fixes, and bashkit docs ([#563](https://github.com/everruns/everruns/pull/563), [#566](https://github.com/everruns/everruns/pull/566), [#567](https://github.com/everruns/everruns/pull/567))

### What's Changed

- feat(load-test): add load testing infrastructure with llmsim and durable race fix ([#568](https://github.com/everruns/everruns/pull/568))
- docs: fix SEO issues across docs site ([#567](https://github.com/everruns/everruns/pull/567))
- docs(ecosystem): add bashkit overview and hide SRE sidebar ([#566](https://github.com/everruns/everruns/pull/566))
- chore(specs): add performance impact guidelines to pre-PR and maintenance checklists ([#565](https://github.com/everruns/everruns/pull/565))
- feat(dashboard): add total sessions count and fix session stats accuracy ([#564](https://github.com/everruns/everruns/pull/564))
- fix(ci): trigger Docker Publish for release tags via workflow_dispatch ([#563](https://github.com/everruns/everruns/pull/563))

## [0.8.0] - 2026-02-21

### Highlights

- **Built-in Skills Discovery** — Skills capability with system prompt integration and Generic harness support ([#516](https://github.com/everruns/everruns/pull/516), [#532](https://github.com/everruns/everruns/pull/532), [#543](https://github.com/everruns/everruns/pull/543))
- **Daytona Integration** — User connection with API key support and official branding ([#522](https://github.com/everruns/everruns/pull/522), [#533](https://github.com/everruns/everruns/pull/533))
- **Generic Harness Type** — New Generic harness with skills, agent_instructions, and copy endpoints ([#512](https://github.com/everruns/everruns/pull/512), [#518](https://github.com/everruns/everruns/pull/518), [#524](https://github.com/everruns/everruns/pull/524))
- **Claude Sonnet 4.6 & Opus 4.6** — New model profiles for latest Claude models ([#531](https://github.com/everruns/everruns/pull/531))
- **Session-Scoped Task Scheduling** — Cron-based scheduled tasks scoped to sessions ([#536](https://github.com/everruns/everruns/pull/536))

### What's Changed

- docs: fix Daytona integration image ([#561](https://github.com/everruns/everruns/pull/561))
- fix(bash): set executable file mode for script execution ([#559](https://github.com/everruns/everruns/pull/559))
- fix(durable): optimize slow claim_due_schedules scheduler query ([#558](https://github.com/everruns/everruns/pull/558))
- chore(deps): update bashkit v0.1.5 → v0.1.6 ([#556](https://github.com/everruns/everruns/pull/556))
- docs: fix duplicate titles, dark-mode logos, and add Braintrust icon ([#557](https://github.com/everruns/everruns/pull/557))
- docs: fix duplicate titles, logo visibility, and redesign home page ([#555](https://github.com/everruns/everruns/pull/555))
- fix(docs): correct edit page link URL for all doc pages ([#554](https://github.com/everruns/everruns/pull/554))
- chore(docs): add Google site verification meta tag ([#553](https://github.com/everruns/everruns/pull/553))
- refactor(migrations): squash post-0.7.0 migrations into single 003_v0.8.0 ([#552](https://github.com/everruns/everruns/pull/552))
- chore: pre-release maintenance — deps, specs, threat model, code cleanup ([#551](https://github.com/everruns/everruns/pull/551))
- feat(capabilities): add features() for UI-driven tab rendering ([#550](https://github.com/everruns/everruns/pull/550))
- fix(ui): fix llm.generation preview modal layout and rename button to View ([#549](https://github.com/everruns/everruns/pull/549))
- refactor(skills): separate AttachSkillCapability (mount-only) from SkillsCapability (discovery+tools) ([#548](https://github.com/everruns/everruns/pull/548))
- docs(skills): add overview video, split into skills + skills-registry ([#547](https://github.com/everruns/everruns/pull/547))
- feat(ui): add frontmatter support to markdown file preview ([#546](https://github.com/everruns/everruns/pull/546))
- chore(deps): bump devalue from 5.6.2 to 5.6.3 in /apps/docs ([#545](https://github.com/everruns/everruns/pull/545))
- fix(durable): gate postgres integration tests behind feature flag ([#544](https://github.com/everruns/everruns/pull/544))
- feat(skills): include first 15 skill descriptions in system prompt ([#543](https://github.com/everruns/everruns/pull/543))
- fix(ui): fix chat input panel and sidebar layout alignment ([#542](https://github.com/everruns/everruns/pull/542))
- feat: add `just start-production` command ([#541](https://github.com/everruns/everruns/pull/541))
- fix(llm): detect model-not-found errors and surface user-friendly message ([#540](https://github.com/everruns/everruns/pull/540))
- docs: add sitemap.xml with lastmod dates ([#539](https://github.com/everruns/everruns/pull/539))
- feat(docs): add Bing meta validation tag, robots.txt with AI crawler rules ([#538](https://github.com/everruns/everruns/pull/538))
- feat(ui): add drag-and-drop file upload to workspace ([#537](https://github.com/everruns/everruns/pull/537))
- feat(schedules): add session-scoped task scheduling ([#536](https://github.com/everruns/everruns/pull/536))
- fix(ui): rename Preview button to View and restore generation visualization ([#535](https://github.com/everruns/everruns/pull/535))
- fix(capabilities): respect /workspace prefix in skills agent-facing paths ([#534](https://github.com/everruns/everruns/pull/534))
- feat(daytona): add official Daytona logo icon and integration docs ([#533](https://github.com/everruns/everruns/pull/533))
- feat(harness): add skills capability to Generic harness ([#532](https://github.com/everruns/everruns/pull/532))
- feat(models): add Claude Sonnet 4.6 and Opus 4.6 model profiles ([#531](https://github.com/everruns/everruns/pull/531))
- refactor(capabilities): make Capability trait async for dynamic system prompt content ([#530](https://github.com/everruns/everruns/pull/530))
- feat(commands): add /ship command for automated ship flow ([#529](https://github.com/everruns/everruns/pull/529))
- fix(auth): auto-refresh expired tokens and preserve page on re-login ([#528](https://github.com/everruns/everruns/pull/528))
- fix(ui): render connection instructions as markdown ([#527](https://github.com/everruns/everruns/pull/527))
- feat(ui): add llm.generation filter to session events ([#526](https://github.com/everruns/everruns/pull/526))
- chore(agents): update pre-PR checklist and add shipping definition ([#525](https://github.com/everruns/everruns/pull/525))
- fix(harness): include agent_instructions in Generic harness ([#524](https://github.com/everruns/everruns/pull/524))
- fix(ui): fix workspace refresh button and auto-refresh on tab switch ([#523](https://github.com/everruns/everruns/pull/523))
- feat(connections): add Daytona user connection with API key support ([#522](https://github.com/everruns/everruns/pull/522))
- feat(ui): reorganize sidebar navigation ([#521](https://github.com/everruns/everruns/pull/521))
- fix(worker): increase control-plane connection timeout from 5s to 30s ([#520](https://github.com/everruns/everruns/pull/520))
- fix(ui): fix settings panel not filling full height ([#519](https://github.com/everruns/everruns/pull/519))
- feat(agents,harnesses): add copy endpoints ([#518](https://github.com/everruns/everruns/pull/518))
- fix(worker): register harness capability tools when agent_id is absent ([#517](https://github.com/everruns/everruns/pull/517))
- feat(capabilities): add built-in skills discovery capability ([#516](https://github.com/everruns/everruns/pull/516))
- feat(ui): add Schedules link to sidebar navigation ([#515](https://github.com/everruns/everruns/pull/515))
- chore(deps): update bashkit from v0.1.4 to v0.1.5 ([#514](https://github.com/everruns/everruns/pull/514))
- fix(seed): upsert seed data with change detection ([#513](https://github.com/everruns/everruns/pull/513))
- feat(harness): rename Default to Base, add Generic harness type ([#512](https://github.com/everruns/everruns/pull/512))
- test: remove 11 ineffective tests ([#511](https://github.com/everruns/everruns/pull/511))

### Migration Notes

**0.7.0 → 0.8.0:** This release includes database schema changes (session-scoped scheduling, Generic harness type, migration squash). A fresh database is required — no automatic migration is supported.

## [0.7.0] - 2026-02-13

### Highlights

- **Skills Registry** — Agent skills registry with top-level navigation and agentskills.io format ([#460](https://github.com/everruns/everruns/pull/460))
- **Harness Abstraction** — New Harness entity between Organization and Agent for flexible grouping ([#434](https://github.com/everruns/everruns/pull/434))
- **Google Gemini Support** — Native Gemini API driver with parametrized LLM integration tests ([#437](https://github.com/everruns/everruns/pull/437))
- **AGENTS.md Support** — New agent_instructions capability for dynamic project instructions ([#449](https://github.com/everruns/everruns/pull/449))
- **Client-Side Tool Calls & Native Images** — Support for client-side tool execution and native image support in tool results ([#443](https://github.com/everruns/everruns/pull/443), [#442](https://github.com/everruns/everruns/pull/442))

### What's Changed

- refactor(migrations): squash SQL migrations to base and durable ([#462](https://github.com/everruns/everruns/pull/462))
- fix: address 3 urgent Linear issues (EVE-5, EVE-6, EVE-8) ([#461](https://github.com/everruns/everruns/pull/461))
- feat(skills): add skills registry with top-level navigation ([#460](https://github.com/everruns/everruns/pull/460))
- fix: rename LINEAR_MCP_API_KEY to LINEAR_API_KEY ([#459](https://github.com/everruns/everruns/pull/459))
- chore: add Linear MCP server configuration ([#458](https://github.com/everruns/everruns/pull/458))
- docs(ui): remove dev-focused sections from Management UI doc ([#457](https://github.com/everruns/everruns/pull/457))
- docs: rename sidebar entries, promote Event Reference, clean up UI doc ([#456](https://github.com/everruns/everruns/pull/456))
- feat(core): wrap system prompt sections in XML tags ([#455](https://github.com/everruns/everruns/pull/455))
- feat(harness): add Harness abstraction between Organization and Agent ([#434](https://github.com/everruns/everruns/pull/434))
- chore(deps): update everruns-sdk to 0.1.2 ([#454](https://github.com/everruns/everruns/pull/454))
- chore(specs): add SDK doc check to maintenance checklist ([#453](https://github.com/everruns/everruns/pull/453))
- chore(deps): upgrade fetchkit to 0.1.1 from crates.io ([#452](https://github.com/everruns/everruns/pull/452))
- chore(specs): align provider type model with app-layer validation ([#451](https://github.com/everruns/everruns/pull/451))
- chore(build): reduce debug binary size and disable incremental in cloud ([#450](https://github.com/everruns/everruns/pull/450))
- feat(core): add agent_instructions capability (AGENTS.md support) ([#449](https://github.com/everruns/everruns/pull/449))
- chore(docs): remove redundant cloud legacy section ([#448](https://github.com/everruns/everruns/pull/448))
- chore(dev): clarify doppler cloud-secret workflow ([#447](https://github.com/everruns/everruns/pull/447))
- feat(test): add SKIP_LLM_INTEGRATION_TESTS_PROVIDERS env var ([#446](https://github.com/everruns/everruns/pull/446))
- feat(auth): pluggable auth backend for SaaS repo support ([#445](https://github.com/everruns/everruns/pull/445))
- fix(ci): handle multiline commit messages in release workflow ([#444](https://github.com/everruns/everruns/pull/444))
- feat(core): add client-side tool calls support ([#443](https://github.com/everruns/everruns/pull/443))
- feat(core): native image support in tool results ([#442](https://github.com/everruns/everruns/pull/442))
- feat(gemini): add Google Gemini API support and parametrize LLM integration tests ([#437](https://github.com/everruns/everruns/pull/437))
- docs: add concepts page with entity diagrams ([#441](https://github.com/everruns/everruns/pull/441))

### Migration Notes

**0.6.0 → 0.7.0:** This release includes database schema changes (Harness abstraction, migration squash). A fresh database is required — no automatic migration is supported.

## [0.6.0] - 2026-02-10

### Highlights

- **Session-Scoped SQL Databases** — Agents can create and query SQLite databases scoped to their session ([#425](https://github.com/everruns/everruns/pull/425))
- **OpenTelemetry Observability** — Full-featured OTel with 13 event types, span hierarchy, and content recording ([#427](https://github.com/everruns/everruns/pull/427))
- **Virtual Bash Capability** — Sandboxed bash execution for agents using bashkit ([#399](https://github.com/everruns/everruns/pull/399))
- **Scheduled Tasks** — Cron-based scheduled task execution for durable workflows ([#405](https://github.com/everruns/everruns/pull/405))
- **Agent Trajectory Visualization** — New UI for visualizing agent execution paths in sessions ([#436](https://github.com/everruns/everruns/pull/436))

### What's Changed

- chore(deps): pre-release maintenance — update deps, specs, and docs ([#439](https://github.com/everruns/everruns/pull/439))
- feat(examples): add HackerNews reader agent example ([#438](https://github.com/everruns/everruns/pull/438))
- feat(ui): agent trajectory visualization in session UI ([#436](https://github.com/everruns/everruns/pull/436))
- chore: add Doppler CLI for secrets management ([#435](https://github.com/everruns/everruns/pull/435))
- chore(deps): update everruns-sdk 0.1→0.1.1 and bashkit v0.1.2→v0.1.4 ([#433](https://github.com/everruns/everruns/pull/433))
- refactor(migrations): squash 6 migrations into 2 logical groups ([#432](https://github.com/everruns/everruns/pull/432))
- chore(specs): add comprehensive threat model with stable IDs ([#431](https://github.com/everruns/everruns/pull/431))
- fix(deps): upgrade llmsim from 0.2.0 to 0.2.1 ([#429](https://github.com/everruns/everruns/pull/429))
- fix(ui): fix workspace file browser display issues ([#428](https://github.com/everruns/everruns/pull/428))
- feat(otel): full-featured OTel with 13 event types, span hierarchy, content recording ([#427](https://github.com/everruns/everruns/pull/427))
- feat(agents): dual-ID pattern with public_id and upsert semantics ([#426](https://github.com/everruns/everruns/pull/426))
- feat(session-sqldb): session-scoped SQL databases ([#425](https://github.com/everruns/everruns/pull/425))
- fix(core): update bashkit to v0.1.2, fix file size in virtual bash ([#424](https://github.com/everruns/everruns/pull/424))
- feat(core): update model profiles for Claude 4.6 and GPT 5.2/5.3 ([#423](https://github.com/everruns/everruns/pull/423))
- feat(ui): replace FileBrowser with AI Elements FileTree ([#422](https://github.com/everruns/everruns/pull/422))
- fix(ci): suppress 'no jobs were run' notifications in release workflow ([#421](https://github.com/everruns/everruns/pull/421))
- fix(ui): remove duplicate Workspace label and fix breadcrumbs ([#419](https://github.com/everruns/everruns/pull/419))
- docs(features): add SDK documentation page ([#417](https://github.com/everruns/everruns/pull/417))
- feat(ui): improve Workspace breadcrumbs visibility ([#415](https://github.com/everruns/everruns/pull/415))
- refactor(test): restructure integration tests with in-process testing and CI optimization ([#395](https://github.com/everruns/everruns/pull/395))
- feat(ci): add UI Jest tests to CI pipeline ([#413](https://github.com/everruns/everruns/pull/413))
- feat(ui): add file previews for Workspace ([#410](https://github.com/everruns/everruns/pull/410))
- feat(durable): add scheduled tasks with cron-based execution ([#405](https://github.com/everruns/everruns/pull/405))
- feat(ui): implement Streamdown for streaming markdown in messages ([#408](https://github.com/everruns/everruns/pull/408))
- fix(api): handle /workspace prefix in filesystem API ([#407](https://github.com/everruns/everruns/pull/407))
- fix(api): accept prefixed EventId for since_id query parameter ([#406](https://github.com/everruns/everruns/pull/406))
- fix(durable): enforce max_attempts when claiming tasks ([#403](https://github.com/everruns/everruns/pull/403))
- test(capabilities): add security limit tests for virtual bash ([#401](https://github.com/everruns/everruns/pull/401))
- feat(capabilities): add virtual bash capability using bashkit ([#399](https://github.com/everruns/everruns/pull/399))
- feat(api): add session-level capabilities configuration ([#396](https://github.com/everruns/everruns/pull/396))
- feat(ci): add CLI e2e tests ([#394](https://github.com/everruns/everruns/pull/394))
- refactor(cli): migrate to everruns-sdk for API client ([#393](https://github.com/everruns/everruns/pull/393))
- fix(example): distinguish local vs example compose containers ([#392](https://github.com/everruns/everruns/pull/392))
- fix(durable): prevent draining workers from claiming new tasks ([#391](https://github.com/everruns/everruns/pull/391))
- fix(ui): remove redundant refresh button from Worker Pool page ([#388](https://github.com/everruns/everruns/pull/388))
- fix(ci): fix release workflow syntax and add manual trigger ([#390](https://github.com/everruns/everruns/pull/390))

### Migration Notes

**0.5.0 → 0.6.0:** This release includes database schema changes (session-scoped SQL databases, migration squash, dual-ID pattern). A fresh database is required — no automatic migration is supported.

## [0.5.0] - 2026-01-30

### Highlights

- **OpenResponses Support** - Added support for [OpenResponses](https://www.openresponses.org/) specification
- **Braintrust Integration** - LLM tracing and observability with Braintrust ([#340](https://github.com/everruns/everruns/pull/340))
- **Simplified API Structure** - Removed org from API paths, now automatically inferred from API key ([#363](https://github.com/everruns/everruns/pull/363))
- **Sessions as Top-Level Entities** - Reworked sessions to be top-level entities under organizations ([#351](https://github.com/everruns/everruns/pull/351))
- **Improved SSE Reliability** - Enhanced durability and graceful disconnect handling for SSE connections ([#387](https://github.com/everruns/everruns/pull/387), [#370](https://github.com/everruns/everruns/pull/370))
- **Automatic Compaction** - Support for `/v1/responses/compact` endpoint with reactive compaction ([#371](https://github.com/everruns/everruns/pull/371))
- **Extended Thinking for Anthropic** - Support for Claude's extended thinking mode with streaming budget tokens ([#338](https://github.com/everruns/everruns/pull/338))

### What's Changed

- fix(durable): graceful SSE disconnect on errors and add comprehensive tests ([#387](https://github.com/everruns/everruns/pull/387))
- chore(deps): update Rust dependencies to latest major versions ([#386](https://github.com/everruns/everruns/pull/386))
- chore(deps): update Rust and UI dependencies ([#385](https://github.com/everruns/everruns/pull/385))
- fix(example): export EVERRUNS_TAG in pull recipe ([#384](https://github.com/everruns/everruns/pull/384))
- fix(durable): populate worker stats in API response ([#383](https://github.com/everruns/everruns/pull/383))
- fix(examples): docker-compose YAML fixes and add image tag option ([#382](https://github.com/everruns/everruns/pull/382))
- feat(ui): adopt oxfmt for JS/TS formatting ([#381](https://github.com/everruns/everruns/pull/381))
- fix(durable): add Resume button for drained workers ([#380](https://github.com/everruns/everruns/pull/380))
- fix(durable): add index for stale task reclaim query ([#379](https://github.com/everruns/everruns/pull/379))
- fix(telemetry): switch from gRPC to HTTP OTLP to fix DNS errors ([#378](https://github.com/everruns/everruns/pull/378))
- fix(ui): wrap EventFilter menu content in DropdownMenuGroup ([#377](https://github.com/everruns/everruns/pull/377))
- feat(example): add pull command to update docker images ([#376](https://github.com/everruns/everruns/pull/376))
- feat(example): add logs command to listen to docker-compose logs ([#375](https://github.com/everruns/everruns/pull/375))
- feat(durable): add fail-rs failure injection testing ([#374](https://github.com/everruns/everruns/pull/374))
- feat(example): pass through OPENAI/ANTHROPIC API keys to docker compose ([#373](https://github.com/everruns/everruns/pull/373))
- feat(core): add comprehensive OpenResponses types module from OpenAPI spec ([#372](https://github.com/everruns/everruns/pull/372))
- feat(llm): add support for /v1/responses/compact endpoint with reactive compaction ([#371](https://github.com/everruns/everruns/pull/371))
- feat(sse): add connection cycling and retry hints for SSE endpoints ([#370](https://github.com/everruns/everruns/pull/370))
- refactor(migrations): squash migrations 003-007 into base schema ([#369](https://github.com/everruns/everruns/pull/369))
- chore(deps): bump next from 16.1.1 to 16.1.5 in /apps/ui ([#368](https://github.com/everruns/everruns/pull/368))
- feat(llm): add automatic retry for rate limit errors ([#367](https://github.com/everruns/everruns/pull/367))
- fix(auth): ensure org cookie is set on auth and UI waits for org initialization ([#366](https://github.com/everruns/everruns/pull/366))
- fix(durable): optimize task claiming query with better index order ([#365](https://github.com/everruns/everruns/pull/365))
- feat(server): auto-apply database migrations on startup ([#364](https://github.com/everruns/everruns/pull/364))
- refactor(api): remove org from API paths, derive from auth context ([#363](https://github.com/everruns/everruns/pull/363))
- refactor: rename MessageRole::Assistant to MessageRole::Agent ([#362](https://github.com/everruns/everruns/pull/362))
- feat(docker): add just example subcommand for docker-compose-full ([#361](https://github.com/everruns/everruns/pull/361))
- feat(ui): unify ID handling with copy buttons ([#360](https://github.com/everruns/everruns/pull/360))
- refactor: rename everruns-control-plane to everruns-server ([#359](https://github.com/everruns/everruns/pull/359))
- feat(ui): add organisation settings page ([#358](https://github.com/everruns/everruns/pull/358))
- feat(events): document events contract and add contract tests ([#357](https://github.com/everruns/everruns/pull/357))
- feat(events): add event filtering with exclude parameter ([#356](https://github.com/everruns/everruns/pull/356))
- feat(ui): implement Slate design system ([#355](https://github.com/everruns/everruns/pull/355))
- feat(core): add in-memory LLM integration tests ([#354](https://github.com/everruns/everruns/pull/354))
- refactor(events): rework event types for input/output symmetry ([#353](https://github.com/everruns/everruns/pull/353))
- feat(ui): show full agent_id instead of truncated version ([#352](https://github.com/everruns/everruns/pull/352))
- refactor(api): make sessions top-level entities under organizations ([#351](https://github.com/everruns/everruns/pull/351))
- fix(scripts): use robust PostgreSQL check instead of port-only check ([#350](https://github.com/everruns/everruns/pull/350))
- chore(ci): add workflow permissions blocks ([#349](https://github.com/everruns/everruns/pull/349))
- feat(core): add InMemoryAgenticLoop and TurnStateMachine ([#348](https://github.com/everruns/everruns/pull/348))
- docs: cleanup AGENTS.md and reorganize documentation ([#347](https://github.com/everruns/everruns/pull/347))
- fix(braintrust): convert message roles to OpenAI-compatible format ([#346](https://github.com/everruns/everruns/pull/346))
- refactor(worker): unify in-memory and durable worker implementations ([#345](https://github.com/everruns/everruns/pull/345))
- fix(observability): fix Braintrust timeline view for reason and act spans ([#344](https://github.com/everruns/everruns/pull/344))
- fix(ui): bypass Next.js proxy for image uploads ([#343](https://github.com/everruns/everruns/pull/343))
- fix(worker): add restart-on-crash logic for worker startup ([#342](https://github.com/everruns/everruns/pull/342))
- refactor(core): use typed IDs throughout codebase for type safety ([#341](https://github.com/everruns/everruns/pull/341))
- feat(observability): add Braintrust integration for LLM tracing ([#340](https://github.com/everruns/everruns/pull/340))
- fix(scripts): improve PostgreSQL detection with /dev/tcp ([#339](https://github.com/everruns/everruns/pull/339))
- feat(core): add extended thinking support for Claude models ([#338](https://github.com/everruns/everruns/pull/338))
- feat(db): add updated_at column to sessions table ([#337](https://github.com/everruns/everruns/pull/337))
- fix(core): isolate event listener errors to prevent crash propagation ([#336](https://github.com/everruns/everruns/pull/336))
- refactor(proto): make ResolveImageResponse and TaskNotification fields required ([#335](https://github.com/everruns/everruns/pull/335))
- fix(worker): add startup retry for control-plane connection ([#334](https://github.com/everruns/everruns/pull/334))
- refactor(core): remove CapabilityId constants and factory methods ([#333](https://github.com/everruns/everruns/pull/333))
- fix(ui): reorder session tabs to Chat, Files, Storage, Events ([#332](https://github.com/everruns/everruns/pull/332))
- fix(scripts): improve Ctrl+C signal handling in start-all/start-dev ([#331](https://github.com/everruns/everruns/pull/331))
- refactor(ui): split components and consolidate utilities ([#330](https://github.com/everruns/everruns/pull/330))
- docs: add OpenAI Platform Traces API to dismissed options ([#329](https://github.com/everruns/everruns/pull/329))
- fix(ui): shorten session ID display with copy button ([#328](https://github.com/everruns/everruns/pull/328))
- chore: code cleanup - centralize utilities, deps, and shared components ([#327](https://github.com/everruns/everruns/pull/327))
- docs(specs): fix API paths to include org prefix and correct event types ([#326](https://github.com/everruns/everruns/pull/326))
- feat(core): add metadata to LLM API requests for tracking ([#325](https://github.com/everruns/everruns/pull/325))
- chore: rename api service to control-plane in docker-compose-full ([#324](https://github.com/everruns/everruns/pull/324))
- docs: rebrand from platform to agentic harness engine ([#323](https://github.com/everruns/everruns/pull/323))
- feat(ui): move info icon to footer row in chat messages ([#322](https://github.com/everruns/everruns/pull/322))
- feat: simplify dev setup for cloud environments ([#321](https://github.com/everruns/everruns/pull/321))
- feat(core): standardize ID schema with Stripe-style prefixed IDs ([#320](https://github.com/everruns/everruns/pull/320))
- fix(scripts): fetch github remote main branch for gh pr merge ([#319](https://github.com/everruns/everruns/pull/319))
- feat(core): add composable message filter abstraction ([#318](https://github.com/everruns/everruns/pull/318))
- feat(ui): add capability settings editor for Docker custom image ([#317](https://github.com/everruns/everruns/pull/317))
- feat(capabilities): add session storage capability with UI ([#316](https://github.com/everruns/everruns/pull/316))
- feat(docker): add docker_logs tool to get container logs ([#315](https://github.com/everruns/everruns/pull/315))
- refactor(ui): rename File System tab to Files with subtitle ([#314](https://github.com/everruns/everruns/pull/314))
- chore: replace dev.sh with just command runner ([#313](https://github.com/everruns/everruns/pull/313))
- feat(ui): simplify chat UI with minimal + icon style ([#312](https://github.com/everruns/everruns/pull/312))
- fix(test): use std::time::Duration in cancel turn test ([#310](https://github.com/everruns/everruns/pull/310))
- fix(api): make cancel turn endpoint idempotent with response body ([#309](https://github.com/everruns/everruns/pull/309))
- feat(metrics): add time-to-first-token tracking for LLM calls ([#308](https://github.com/everruns/everruns/pull/308))
- feat(openai): adopt Open Responses API as default, remove Azure support ([#307](https://github.com/everruns/everruns/pull/307))
- feat(capabilities): add capability dependencies with automatic resolution ([#306](https://github.com/everruns/everruns/pull/306))
- fix(ui): use TooltipPrimitive.Provider for tooltips to work ([#305](https://github.com/everruns/everruns/pull/305))
- feat(session): add cancel turn functionality ([#304](https://github.com/everruns/everruns/pull/304))
- feat(dev): auto-check UI dependencies in start-all and start-dev ([#303](https://github.com/everruns/everruns/pull/303))
- feat(events): include system message in llm.generation event ([#302](https://github.com/everruns/everruns/pull/302))
- feat(capabilities): add experimental Docker container capability ([#301](https://github.com/everruns/everruns/pull/301))
- chore: update default model to gpt-5.2 ([#300](https://github.com/everruns/everruns/pull/300))
- feat(ui): add pagination and auto-refresh to session events ([#299](https://github.com/everruns/everruns/pull/299))
- fix(api): use cached tools when viewing MCP capability details ([#298](https://github.com/everruns/everruns/pull/298))
- fix(deps): update llmsim to 0.2.0 to fix dependency vulnerabilities ([#297](https://github.com/everruns/everruns/pull/297))
- feat(llm): add user-friendly error for request-too-large errors ([#296](https://github.com/everruns/everruns/pull/296))
- feat(models): add automatic model discovery from LLM provider APIs ([#295](https://github.com/everruns/everruns/pull/295))
- feat(ui): auto-focus message input when session loads ([#294](https://github.com/everruns/everruns/pull/294))
- feat(ui): add real-time streaming chat with thinking indicator ([#293](https://github.com/everruns/everruns/pull/293))
- ci: add lockfile check to verify Cargo.lock is up to date ([#292](https://github.com/everruns/everruns/pull/292))
- feat(ui): add LLM message history visualization component ([#291](https://github.com/everruns/everruns/pull/291))
- feat(ui): add markdown support for capability descriptions ([#290](https://github.com/everruns/everruns/pull/290))
- chore: update lock files and document release requirements ([#289](https://github.com/everruns/everruns/pull/289))
- fix(ci): quote if expression in release workflow ([#288](https://github.com/everruns/everruns/pull/288))
- refactor(capabilities): replace webfetch with fetchkit library ([#279](https://github.com/everruns/everruns/pull/279))
- feat(skill): update ui-screenshots to use agent-browser ([#197](https://github.com/everruns/everruns/pull/197))

### Migration Notes

**0.4.0 → 0.5.0:** No backward compatibility. This release includes schema changes (migrations squashed) and API path changes (org removed from paths). Export agents via API, reset database, re-import.

## [0.4.0] - 2025-01-17

### Highlights

- **Organization-scoped Multitenancy** - Full tenant isolation with organization-based resource scoping
- **MCP Support** - Model Context Protocol integration (without Auth)
- **Push-based Work Scheduling** - Real-time task distribution replacing polling for durable execution
- **DEV_MODE** - Run without PostgreSQL using in-memory storage for quick development
- **Multimodality support for images** - Attach and process multiple images in messages

### What's Changed

- fix(ci): update Docker tag strategy for stable latest ([#287](https://github.com/everruns/everruns/pull/287)) by [@chaliy](https://github.com/chaliy)
- refactor: remove outdated decision comments from Cargo.toml ([#285](https://github.com/everruns/everruns/pull/285)) by [@chaliy](https://github.com/chaliy)
- fix(deps): address security vulnerabilities ([#284](https://github.com/everruns/everruns/pull/284)) by [@chaliy](https://github.com/chaliy)
- refactor(db): squash migrations into two files ([#283](https://github.com/everruns/everruns/pull/283)) by [@chaliy](https://github.com/chaliy)
- feat(release): add automated release workflow with CHANGELOG.md as source of truth ([#282](https://github.com/everruns/everruns/pull/282)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add capability mounting to session filesystem ([#281](https://github.com/everruns/everruns/pull/281)) by [@chaliy](https://github.com/chaliy)
- feat(tests): add agent integration tests for tool calls across providers ([#280](https://github.com/everruns/everruns/pull/280)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add agent preview mode to show final agent shape ([#276](https://github.com/everruns/everruns/pull/276)) by [@chaliy](https://github.com/chaliy)
- docs: refactor README for users, move dev setup to CONTRIBUTING ([#278](https://github.com/everruns/everruns/pull/278)) by [@chaliy](https://github.com/chaliy)
- feat: implement organization-scoped multitenancy ([#277](https://github.com/everruns/everruns/pull/277)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP agent support with virtual capabilities ([#275](https://github.com/everruns/everruns/pull/275)) by [@chaliy](https://github.com/chaliy)
- feat(images): add multi-image attachment support for messages ([#274](https://github.com/everruns/everruns/pull/274)) by [@chaliy](https://github.com/chaliy)
- refactor: remove database trigger, implement usage tracking in Rust ([#273](https://github.com/everruns/everruns/pull/273)) by [@chaliy](https://github.com/chaliy)
- fix: remove outdated Temporal reference from dev.sh ([#272](https://github.com/everruns/everruns/pull/272)) by [@chaliy](https://github.com/chaliy)
- feat(worker): implement push-based task notifications ([#270](https://github.com/everruns/everruns/pull/270)) by [@chaliy](https://github.com/chaliy)
- feat(durable): add PostgreSQL-backed load test benchmarks ([#268](https://github.com/everruns/everruns/pull/268)) by [@chaliy](https://github.com/chaliy)
- fix(worker): reduce poll interval from 1s to 100ms ([#262](https://github.com/everruns/everruns/pull/262)) by [@chaliy](https://github.com/chaliy)
- feat(models): add favorite LLM models support ([#265](https://github.com/everruns/everruns/pull/265)) by [@chaliy](https://github.com/chaliy)
- feat(durable): integrate circuit breaker for LLM provider protection ([#263](https://github.com/everruns/everruns/pull/263)) by [@chaliy](https://github.com/chaliy)
- feat(ui): update session status and usage via SSE in real-time ([#258](https://github.com/everruns/everruns/pull/258)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add llmsim provider support and seed data ([#261](https://github.com/everruns/everruns/pull/261)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add per-agent capability configuration ([#260](https://github.com/everruns/everruns/pull/260)) by [@chaliy](https://github.com/chaliy)
- fix(dev): fix dev mode LLM errors and improve DX ([#259](https://github.com/everruns/everruns/pull/259)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove max-width constraint from agent edit page ([#256](https://github.com/everruns/everruns/pull/256)) by [@chaliy](https://github.com/chaliy)
- fix(ui): maintain input focus after sending chat message ([#257](https://github.com/everruns/everruns/pull/257)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): replace durable polling with SSE streaming ([#254](https://github.com/everruns/everruns/pull/254)) by [@chaliy](https://github.com/chaliy)
- feat: add LLM token usage tracking and visualization ([#250](https://github.com/everruns/everruns/pull/250)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add SessionCard component with status display and info button ([#247](https://github.com/everruns/everruns/pull/247)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP server registration and management ([#246](https://github.com/everruns/everruns/pull/246)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add message info icon with metadata tooltip ([#243](https://github.com/everruns/everruns/pull/243)) by [@chaliy](https://github.com/chaliy)
- feat(api,ui): add pagination to sessions API ([#244](https://github.com/everruns/everruns/pull/244)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add centralized capability icons ([#237](https://github.com/everruns/everruns/pull/237)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.3.x → 0.4.0:** No automatic migration. This release includes schema changes for multitenancy and capabilities. Export agents via API, reset database, re-import.

## [0.3.0] - 2025-01-09

### Highlights

- **Durable Execution Engine** - Custom PostgreSQL-backed workflow engine replacing Temporal
- **CLI Tool** - Command-line interface for agent and session management
- **OpenTelemetry Integration** - Distributed tracing with gen-ai semantic conventions
- **SSE Events** - Real-time session status updates replacing polling

### What's Changed

- feat(durable): add custom durable execution engine (Phases 1-4) ([#154](https://github.com/everruns/everruns/pull/154)) by [@chaliy](https://github.com/chaliy)
- refactor(telemetry): implement event-listener-based OTel with gen-ai semantic conventions ([#161](https://github.com/everruns/everruns/pull/161)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add everruns CLI for agent and session management ([#163](https://github.com/everruns/everruns/pull/163)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add auto-generated API Reference from OpenAPI spec ([#164](https://github.com/everruns/everruns/pull/164)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add fake demo tools and agents for warehouse, AWS, CRM, and financial operations ([#168](https://github.com/everruns/everruns/pull/168)) by [@chaliy](https://github.com/chaliy)
- feat(api): add agent import/export endpoints ([#172](https://github.com/everruns/everruns/pull/172)) by [@chaliy](https://github.com/chaliy)
- feat(api): add input validation for agent create/update/import ([#173](https://github.com/everruns/everruns/pull/173)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add seed agent markdown files and upload-agents command ([#176](https://github.com/everruns/everruns/pull/176)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.2.x → 0.3.0:** No automatic migration. Export agents via API, reset database, re-import.

## [0.2.0] - 2024-12

### Highlights

- **Temporal Integration** - Workflow orchestration via Temporal
- **PostgreSQL Storage** - Database layer with SQLx
- **Management UI** - Next.js dashboard for agent management

### What's Changed

- Initial implementation with Temporal-based workflow orchestration
- Complete rewrite from early POC architecture

### Migration Notes

**0.1.x → 0.2.0:** Complete rewrite. Manual migration required.

## [0.1.0] - 2024-11

### Highlights

- Initial proof-of-concept release
- Basic agent execution with simple message handling

---

## Versioning Policy

- **Major versions** (1.0, 2.0): Breaking API changes, architectural shifts
- **Minor versions** (0.3, 0.4): New features, schema changes requiring fresh DB
- **Patch versions** (0.3.1): Bug fixes, no schema changes
