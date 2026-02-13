# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **⚠️ Important:** There is no automatic migration between versions. Each major/minor release requires a fresh database. Back up any data you need before upgrading.

## [Unreleased]

<!-- New changes go here. Use `/prepare-release X.Y.Z` to generate draft from commits. -->

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
